//! Which members of a package build read which other members' interfaces
//! (v0.8, cross-module native calls).
//!
//! A member reads the interface of every member it imports from at module
//! level (a `from M import ...` at top level or in a module-level `if`/`try`
//! block, but not under `if TYPE_CHECKING:`, which never runs), unless the
//! two import each other, directly or through others: the members of one
//! cycle (one strongly connected component of the import graph) keep
//! importing each other through CPython only, so every member's producers
//! come before it and each interface is written before any member that
//! reads it starts. A function-local import is not an edge (it runs after
//! the module's body), so it never makes a cycle.

use super::PackageModule;
use super::siblings::{imported_module, module_imports_where};
use pycc_ast::{Expr, Stmt};

/// Whether `test` is `TYPE_CHECKING` or `<module>.TYPE_CHECKING`.
fn is_type_checking(test: &Expr) -> bool {
    match test {
        Expr::Name(name) => name.id.as_str() == "TYPE_CHECKING",
        Expr::Attribute(attr) => attr.attr.as_str() == "TYPE_CHECKING",
        _ => false,
    }
}

/// For each of `modules` (whose sources are `sources`), the indices of the
/// members whose interfaces it reads, in ascending order.
pub(crate) fn producers(modules: &[PackageModule], sources: &[String]) -> Vec<Vec<usize>> {
    let index: std::collections::HashMap<&str, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, module)| (module.dotted.as_str(), i))
        .collect();
    let edges: Vec<Vec<usize>> = modules
        .iter()
        .zip(sources)
        .enumerate()
        .map(|(i, (module, source))| {
            let Ok(parsed) = pycc_parser::parse_all(source) else {
                return Vec::new();
            };
            let mut imports = Vec::new();
            module_imports_where(
                &parsed.body,
                &mut imports,
                &|stmt: &Stmt| !matches!(stmt, Stmt::If(node) if is_type_checking(&node.test)),
            );
            let mut targets: Vec<usize> = imports
                .iter()
                .filter_map(|import| {
                    imported_module(
                        &module.dotted,
                        module.is_package,
                        import.level,
                        import.module.as_ref().map(|m| m.as_str()),
                    )
                })
                .filter_map(|dotted| index.get(dotted.as_str()).copied())
                .filter(|target| *target != i)
                .collect();
            targets.sort_unstable();
            targets.dedup();
            targets
        })
        .collect();
    let component = components(&edges);
    edges
        .iter()
        .enumerate()
        .map(|(i, targets)| {
            targets
                .iter()
                .copied()
                .filter(|target| component[*target] != component[i])
                .collect()
        })
        .collect()
}

/// The strongly connected component of each node of `edges` (Tarjan's
/// algorithm, iterative).
fn components(edges: &[Vec<usize>]) -> Vec<usize> {
    let n = edges.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut component = vec![usize::MAX; n];
    let mut next_index = 0;
    let mut next_component = 0;
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        // (node, position in its edge list)
        let mut work = vec![(root, 0)];
        index[root] = next_index;
        low[root] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (node, ref mut position)) = work.last_mut() {
            if let Some(&target) = edges[node].get(*position) {
                *position += 1;
                if index[target] == usize::MAX {
                    index[target] = next_index;
                    low[target] = next_index;
                    next_index += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    work.push((target, 0));
                } else if on_stack[target] {
                    low[node] = low[node].min(index[target]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == index[node] {
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    component[member] = next_component;
                    if member == node {
                        break;
                    }
                }
                next_component += 1;
            }
        }
    }
    component
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(dotted: &str, is_package: bool) -> PackageModule {
        PackageModule {
            dotted: dotted.to_string(),
            relative: String::new(),
            path: Default::default(),
            is_package,
        }
    }

    #[test]
    fn members_read_the_interfaces_of_what_they_import_outside_cycles() {
        let modules = [
            module("p", true),
            module("p.a", false),
            module("p.b", false),
            module("p.c", false),
            module("p.d", false),
        ];
        let sources = [
            "from .a import f\nfrom .b import g\n".to_string(),
            "from .c import h\nfrom typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    from .d \
             import x\n"
                .to_string(),
            "from p.c import h\nfrom . import a\ndef f():\n    from .d import y\n".to_string(),
            "from .b import g\nfrom .c import self_import\n".to_string(),
            "from .a import f\n".to_string(),
        ];
        let producers = producers(&modules, &sources);
        // `p.b` and `p.c` import each other, `p.b` imports `p` (`from .
        // import a`), which imports `p.a` and `p.b`, and `p.a` imports
        // `p.c`: one cycle of four, no interfaces among them; `p.d` reads
        // `p.a`'s.
        assert_eq!(producers, [vec![], vec![], vec![], vec![], vec![1]]);
        // Without the cycle back to the package, the order is a chain.
        let sources = [
            "from .a import f\n".to_string(),
            "from .c import h\n".to_string(),
            "x = 1\n".to_string(),
            "y = 1\n".to_string(),
            "from .a import f\nfrom .b import x\n".to_string(),
        ];
        assert_eq!(
            super::producers(&modules, &sources),
            [vec![1], vec![3], vec![], vec![], vec![1, 2]]
        );
    }

    #[test]
    fn components_group_cycles() {
        let component = components(&[vec![1], vec![2], vec![0], vec![2], vec![]]);
        assert_eq!(component[0], component[1]);
        assert_eq!(component[1], component[2]);
        assert_ne!(component[3], component[0]);
        assert_ne!(component[4], component[0]);
    }
}
