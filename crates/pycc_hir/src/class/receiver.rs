//! The instance-method receiver decision ([#1181](https://github.com/rotnov/pycc/issues/1181)).
//!
//! Extracted from `class.rs` per AGENTS.md's "Keep source files
//! decomposable" rule and D-185's per-pull-request narrowing of
//! [#548](https://github.com/rotnov/pycc/issues/548): everything that
//! answers "which parameter is the receiver, and is this method's receiver
//! acceptable?" for a *regular* instance method (including a `@property`
//! getter, a `@<name>.setter` setter, and an `@abstractmethod`) lives here.
//! `@classmethod`'s own `cls` rule and `@staticmethod`'s receiver-less
//! shape keep their own arms in `class.rs`'s method-kind `match`.
//!
//! THE RULE, stated once here:
//!
//! > An instance method's receiver is its **first positional parameter**,
//! > whatever identifier it is spelled with -- Python does not reserve
//! > `self`, it is a PEP 8 convention. The receiver is *lowered* under the
//! > canonical name `self` ([`super::lower_method`] writes it), and when the
//! > source spells it anything else the lowered body opens with an alias
//! > statement (`<name> = self`, [`alias_stmt`]) so the user's spelling is
//! > an ordinary local bound to the canonical receiver.
//!
//! Keeping the canonical name is what makes every downstream consumer that
//! keys on the name `self` -- `pycc_types::Environment`'s verbatim parameter
//! binding, `pycc_types::class::super_call`'s `binding_state("self")` test,
//! `pycc_mir::class`'s `lookup(scopes, "self")` -- correct without a change.
//! The aliasing is *invisible* for every program the compiler accepts,
//! because two guards ([`check_renamed_receiver`]) reject the only shapes
//! that could observe it:
//!
//! 1. **No occurrence of `self`.** Since the canonical parameter still
//!    exists, a method written `def m(this)` would otherwise accept a body
//!    mentioning `self`, which CPython raises `NameError` for (or, for a
//!    module-level global named `self`, resolves to the global -- pycc would
//!    resolve the parameter instead). Any read *or* binding of the
//!    identifier `self` anywhere in the method is rejected.
//! 2. **No rebinding of the receiver.** CPython's zero-argument `super()`
//!    reads the *current* value of the frame's first local, so a method that
//!    reassigns its receiver and then calls `super()` binds to the new
//!    value. The alias cannot reproduce that: `pycc_mir` looks up the
//!    canonical `self` parameter, which assigning to the alias leaves
//!    untouched. Rather than silently answering `1` where CPython answers
//!    `7`, any rebinding of the receiver name is rejected. This is a
//!    deliberate, documented narrowing of "usable wherever `self` is usable
//!    today"; it can be lifted by whichever future shape carries the source
//!    spelling into the parameter list itself.
//!
//! Both guards scan the **raw AST**, not the lowered HIR: shapes such as
//! `lambda` never lower at all, so a HIR scan would under-cover. Both run
//! *before* `stmt::lower_body`, because `global` and `nonlocal` each report
//! their own `C0001` from that pass and a scan placed after it would never
//! reach those arms with the receiver's own message. A `del` of the receiver
//! is refused by its own scan, [`check_receiver_not_deleted`] (#1244).

use pycc_ast::visitor::{self, Visitor};
use pycc_ast::{Expr, ParameterWithDefault, Parameters, Stmt, StmtFunctionDef};
use pycc_diag::Diagnostic;

use crate::HirStmt;
use crate::unsupported;

use super::MethodKind;

/// The canonical HIR name of an instance method's receiver parameter. The
/// source may spell it anything; `lower_method` always lowers it under this
/// name -- see this module's own doc comment for why that is load-bearing.
pub(super) const CANONICAL_RECEIVER: &str = "self";

/// The three dunders CPython *implicitly rebinds* to a different method
/// kind: `__new__` becomes a static method, `__init_subclass__` and
/// `__class_getitem__` become class methods. pycc models none of them yet,
/// and all three are rejected today -- by the very receiver-spelling rule
/// #1181 removes (`def __new__(x)`, `def __init_subclass__(cls)`,
/// `def __class_getitem__(i: int)` each reported the old `C0001`). Relaxing
/// the spelling rule without this guard would silently turn each into a
/// plain *instance* method: a fresh CPython deviation introduced by a change
/// whose whole point is removing one. The guard preserves the status quo and
/// changes no currently-accepted program, since the `self`-spelled twin of
/// each is accepted today and stays accepted.
const IMPLICITLY_REBOUND_DUNDERS: [&str; 3] = ["__new__", "__init_subclass__", "__class_getitem__"];

/// The receiver parameter of a regular instance method, plus the parameters
/// that follow it, split the way PEP 570 (#383) requires: the receiver is
/// the first parameter *overall*, which lives in `posonlyargs` when a `/`
/// marker follows it and in `args` otherwise.
pub(super) struct ReceiverSplit<'p> {
    /// The receiver parameter itself.
    pub(super) receiver: &'p ParameterWithDefault,
    /// The positional-only parameters that follow the receiver.
    pub(super) posonly_rest: &'p [ParameterWithDefault],
    /// The ordinary parameters that follow the positional-only ones.
    pub(super) args_rest: &'p [ParameterWithDefault],
}

impl ReceiverSplit<'_> {
    /// The receiver's spelling as written in the source. `collect_init_attrs`
    /// compares an `__init__` body's `<receiver>.<attr> = ...` base against
    /// this, not against the literal `self`: without that,
    /// `def __init__(this): this.v = 1` establishes zero attribute slots and
    /// every later read fails with `T0044`.
    pub(super) fn name(&self) -> &str {
        self.receiver.parameter.name.as_str()
    }

    /// How many parameters the method takes besides the receiver -- the
    /// arity the `@property`/`@<name>.setter` rules constrain.
    pub(super) fn extra_count(&self) -> usize {
        self.posonly_rest.len() + self.args_rest.len()
    }
}

/// Splits a regular instance method's receiver off its parameter list, or
/// rejects a method that declares no parameter at all.
pub(super) fn split_receiver<'p>(
    parameters: &'p Parameters,
    range: std::ops::Range<u32>,
) -> Result<ReceiverSplit<'p>, Diagnostic> {
    // PEP 570 (#383): the receiver is the first parameter overall -- it may
    // be in `posonlyargs` (if `/` follows it) or in `args`.
    if let Some((receiver, posonly_rest)) = parameters.posonlyargs.split_first() {
        return Ok(ReceiverSplit {
            receiver,
            posonly_rest,
            args_rest: parameters.args.as_slice(),
        });
    }
    if let Some((receiver, args_rest)) = parameters.args.split_first() {
        return Ok(ReceiverSplit {
            receiver,
            posonly_rest: &[],
            args_rest,
        });
    }
    Err(unsupported(
        "a method must take a receiver as its first parameter",
        range,
    ))
}

/// The structural rejections that apply to an instance method's receiver
/// whatever it is spelled: it may carry neither a default value nor an
/// explicit type annotation. #1181 keeps both rejections and only rewords
/// them, so they no longer assert the parameter is called `self`.
pub(super) fn check_receiver_param(
    split: &ReceiverSplit<'_>,
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    if split.receiver.default.is_some() {
        return Err(unsupported(
            "a method's receiver parameter cannot have a default value",
            range,
        ));
    }
    if split.receiver.parameter.annotation.is_some() {
        return Err(unsupported(
            "an explicit type annotation on a method's receiver parameter is not supported yet",
            range,
        ));
    }
    Ok(())
}

/// #377: a `@property` getter takes only the receiver (no additional
/// parameters); a `@<name>.setter` setter takes exactly one additional
/// parameter (the value to assign). A regular method has no arity constraint
/// beyond [`check_receiver_param`]'s structural checks.
pub(super) fn check_property_arity(
    kind: &MethodKind,
    split: &ReceiverSplit<'_>,
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    let extra_count = split.extra_count();
    match kind {
        MethodKind::PropertyGetter { .. } if extra_count > 0 => Err(unsupported(
            "a `@property` getter must take only its receiver (no additional parameters)",
            range,
        )),
        MethodKind::PropertySetter { .. } if extra_count != 1 => Err(unsupported(
            "a `@<name>.setter` setter must take exactly one parameter besides its receiver",
            range,
        )),
        _ => Ok(()),
    }
}

/// The two #1181 guards plus the implicitly-rebound-dunder guard, all of
/// which apply *only* when the source spells the receiver something other
/// than the canonical `self`. A `self`-spelled receiver reaches none of them
/// and so keeps exactly its pre-#1181 behavior.
///
/// See this module's own doc comment for why each guard exists and what it
/// costs. Ordering is deliberate: the dunder guard names a more specific
/// reason than either occurrence scan, so it reports first.
pub(super) fn check_renamed_receiver(
    def: &StmtFunctionDef,
    method_name: &str,
    split: &ReceiverSplit<'_>,
) -> Result<(), Diagnostic> {
    let receiver_name = split.name();
    if receiver_name == CANONICAL_RECEIVER {
        return Ok(());
    }
    if IMPLICITLY_REBOUND_DUNDERS.contains(&method_name) {
        return Err(unsupported(
            format!(
                "`{method_name}` must spell its first parameter `self`: CPython implicitly \
                 rebinds it to a `@staticmethod` (`__new__`) or a `@classmethod` \
                 (`__init_subclass__`, `__class_getitem__`), which pycc does not model yet, \
                 so accepting another spelling here would silently compile it as a plain \
                 instance method"
            ),
            def.parameters.range,
        ));
    }
    if declares_parameter_named(&def.parameters, CANONICAL_RECEIVER)
        || mentions_identifier(&def.body, CANONICAL_RECEIVER)
    {
        return Err(unsupported(
            format!(
                "a method whose receiver is spelled `{receiver_name}` cannot also use the name \
                 `self`: `self` is the receiver's canonical binding inside the compiler, so a \
                 read of it here would resolve to the receiver rather than to the module-level \
                 global or local the source means -- spell the receiver `self` instead, or \
                 rename the other binding"
            ),
            def.parameters.range,
        ));
    }
    if rebinds_identifier(&def.body, receiver_name) {
        return Err(unsupported(
            format!(
                "rebinding a method's receiver (`{receiver_name}`) is not supported yet: \
                 CPython's zero-argument `super()` reads the receiver slot's current value, \
                 while pycc binds the receiver once at entry -- so a rebinding method would \
                 silently answer with the original receiver. Spell the receiver `self` and \
                 assign to a differently named local instead"
            ),
            def.parameters.range,
        ));
    }
    Ok(())
}

/// #1244: refuses a `del` of a method's receiver (`self` or its renamed
/// spelling for an instance method, `cls` for a `@classmethod`) anywhere in
/// the method body. CPython's zero-argument `super()` reads the receiver
/// slot and raises once it is deleted, while pycc binds the canonical
/// receiver once at entry, so a deleting method could silently answer with
/// the original receiver. A `@staticmethod` has no receiver and is not
/// checked. Runs before [`check_renamed_receiver`]'s early return for a
/// `self`-spelled receiver, so it covers both spellings.
pub(crate) fn check_receiver_not_deleted(
    body: &[Stmt],
    receiver_name: &str,
    range: std::ops::Range<u32>,
) -> Result<(), Diagnostic> {
    struct DeleteScan<'t> {
        target: &'t str,
        found: bool,
    }
    impl<'a> Visitor<'a> for DeleteScan<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::Delete(del) = stmt {
                for target in &del.targets {
                    crate::stmt::del::collect_target_names(target, &mut |name| {
                        self.found |= name == self.target;
                    });
                }
            }
            visitor::walk_stmt(self, stmt);
        }
    }
    let mut scan = DeleteScan {
        target: receiver_name,
        found: false,
    };
    scan.visit_body(body);
    if scan.found {
        return Err(unsupported(
            format!(
                "a `del` of a method's receiver (`{receiver_name}`) is not supported: \
                 CPython's zero-argument `super()` reads the receiver slot, while pycc \
                 binds the receiver once at entry"
            ),
            range,
        ));
    }
    Ok(())
}

/// The alias statement prepended to a lowered method body whose source
/// receiver is spelled something other than [`CANONICAL_RECEIVER`]:
/// `<name> = self`. Binding the user's spelling as an ordinary local is what
/// lets the canonical parameter name stay in place -- see this module's own
/// doc comment.
pub(super) fn alias_stmt(receiver_name: &str) -> HirStmt {
    HirStmt::Assign {
        target: receiver_name.to_string(),
        value: crate::HirExpr::Name(CANONICAL_RECEIVER.to_string()),
    }
}

/// Whether the method declares a parameter (other than the receiver) spelled
/// `target`. A second parameter named `self` alongside a renamed receiver
/// would collide with the canonical parameter name the receiver is lowered
/// under, so the occurrence guard has to see the parameter list, not just
/// the body.
fn declares_parameter_named(parameters: &Parameters, target: &str) -> bool {
    // Only `posonlyargs` and `args` are walked, and the first of them (the
    // receiver itself) is skipped: `lower_method` has already rejected
    // `*args`, `**kwargs` and every keyword-only parameter with their own
    // `C0001` before this scan can run, so no other parameter list exists.
    parameters
        .posonlyargs
        .iter()
        .chain(parameters.args.iter())
        .skip(1)
        .any(|param| param.parameter.name.as_str() == target)
}

/// Whether `target` occurs anywhere in `body`, in a read position or in any
/// binding position. Guard 1's scan.
fn mentions_identifier(body: &[Stmt], target: &str) -> bool {
    scan(body, target, true)
}

/// Whether `target` is *bound* anywhere in `body`. Guard 2's scan: reads do
/// not count, so an ordinary use of the receiver is untouched.
fn rebinds_identifier(body: &[Stmt], target: &str) -> bool {
    scan(body, target, false)
}

/// The shared scan behind both guards. The binding-position override set is
/// `dunder_name.rs`'s, extended with `global`/`nonlocal` identifiers (which
/// that module has no arm for, since it scans module scope) -- every
/// statement form that stores a name, plus `visit_pattern` for a `match`
/// capture (an `Identifier`, invisible to `visit_expr`), `visit_except_handler`
/// for `except ... as`, and the walrus. `include_reads` additionally counts
/// every `Expr::Name`, which is what makes guard 1 a full occurrence scan
/// -- and what makes it correctly *exclude* an attribute named `.self` and a
/// keyword argument `f(self=1)`, since each carries an `Identifier` rather
/// than an `Expr::Name`.
///
/// A `del <target>` is an `Expr::Name` and so counts as an occurrence for
/// guard 1; it is deliberately *not* a binding for guard 2, because
/// [`check_receiver_not_deleted`] refuses a `del` of the receiver (whatever
/// its spelling, and before this guard runs) with its own message (#1244).
fn scan(body: &[Stmt], target: &str, include_reads: bool) -> bool {
    struct IdentScan<'t> {
        target: &'t str,
        include_reads: bool,
        found: bool,
    }
    impl IdentScan<'_> {
        fn record(&mut self, bound: bool) {
            if bound {
                self.found = true;
            }
        }
        fn binds_target(&self, expr: &Expr) -> bool {
            match expr {
                Expr::Name(name) => name.id.as_str() == self.target,
                Expr::Tuple(tuple) => tuple.elts.iter().any(|elt| self.binds_target(elt)),
                Expr::List(list) => list.elts.iter().any(|elt| self.binds_target(elt)),
                Expr::Starred(starred) => self.binds_target(&starred.value),
                _ => false,
            }
        }
        fn alias_binds_target(&self, alias: &pycc_ast::Alias) -> bool {
            match &alias.asname {
                Some(asname) => asname.as_str() == self.target,
                None => alias
                    .name
                    .as_str()
                    .split('.')
                    .next()
                    .is_some_and(|root| root == self.target),
            }
        }
    }
    impl<'a> Visitor<'a> for IdentScan<'_> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if self.found {
                return;
            }
            match stmt {
                Stmt::FunctionDef(function_def) => {
                    self.record(function_def.name.as_str() == self.target);
                }
                Stmt::ClassDef(class_def) => {
                    self.record(class_def.name.as_str() == self.target);
                }
                Stmt::TypeAlias(type_alias) => {
                    self.record(self.binds_target(&type_alias.name));
                }
                Stmt::AnnAssign(ann_assign) => {
                    self.record(
                        ann_assign.value.is_some() && self.binds_target(&ann_assign.target),
                    );
                }
                Stmt::AugAssign(aug_assign) => {
                    self.record(self.binds_target(&aug_assign.target));
                }
                Stmt::Assign(assign) => {
                    self.record(assign.targets.iter().any(|t| self.binds_target(t)));
                }
                Stmt::For(for_stmt) => {
                    self.record(self.binds_target(&for_stmt.target));
                }
                Stmt::With(with_stmt) => {
                    self.record(
                        with_stmt
                            .items
                            .iter()
                            .filter_map(|item| item.optional_vars.as_deref())
                            .any(|t| self.binds_target(t)),
                    );
                }
                Stmt::Import(import) => {
                    self.record(import.names.iter().any(|a| self.alias_binds_target(a)));
                }
                Stmt::ImportFrom(import) => {
                    self.record(import.names.iter().any(|a| self.alias_binds_target(a)));
                }
                Stmt::Global(global) => {
                    self.record(global.names.iter().any(|n| n.as_str() == self.target));
                }
                Stmt::Nonlocal(nonlocal) => {
                    self.record(nonlocal.names.iter().any(|n| n.as_str() == self.target));
                }
                _ => {}
            }
            if self.found {
                return;
            }
            visitor::walk_stmt(self, stmt);
        }

        fn visit_except_handler(&mut self, handler: &'a pycc_ast::ExceptHandler) {
            if self.found {
                return;
            }
            let pycc_ast::ExceptHandler::ExceptHandler(handler_inner) = handler;
            self.record(
                handler_inner
                    .name
                    .as_ref()
                    .is_some_and(|name| name.as_str() == self.target),
            );
            if self.found {
                return;
            }
            visitor::walk_except_handler(self, handler);
        }

        fn visit_pattern(&mut self, pattern: &'a pycc_ast::Pattern) {
            if self.found {
                return;
            }
            // `walk_pattern` recurses through nested patterns but never
            // surfaces a capture *identifier*, which is an `Identifier`
            // rather than an `Expr::Name` and so is invisible to
            // `visit_expr` -- the finding that makes a reads-only scan
            // unsound (a `case self:` capture would rebind the canonical
            // receiver and `super()` would then read the rebound value).
            let captured = match pattern {
                pycc_ast::Pattern::MatchAs(as_pattern) => as_pattern.name.as_ref(),
                pycc_ast::Pattern::MatchStar(star_pattern) => star_pattern.name.as_ref(),
                pycc_ast::Pattern::MatchMapping(mapping_pattern) => mapping_pattern.rest.as_ref(),
                _ => None,
            };
            self.record(captured.is_some_and(|name| name.as_str() == self.target));
            if self.found {
                return;
            }
            visitor::walk_pattern(self, pattern);
        }

        fn visit_expr(&mut self, expr: &'a Expr) {
            if self.found {
                return;
            }
            match expr {
                Expr::Named(named) => {
                    self.record(self.binds_target(&named.target));
                }
                Expr::Name(name) => {
                    self.record(self.include_reads && name.id.as_str() == self.target);
                }
                _ => {}
            }
            if self.found {
                return;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut scan = IdentScan {
        target,
        include_reads,
        found: false,
    };
    scan.visit_body(body);
    scan.found
}
