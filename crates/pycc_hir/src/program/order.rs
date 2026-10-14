//! Execution order of a linked program (v0.3 accept 1): each imported
//! module's items are spliced into its first importer at the position of
//! the `import` statement that first ran it, so a module body runs once, in
//! CPython's import order, before the importer's next statement -- not
//! hoisted ahead of the importer's whole body.
//!
//! [`Placement`] says where an input goes; an input with none is a root,
//! emitted in input order (the entry, and every input of a caller that
//! places nothing, which keeps the plain concatenation order). A foreign
//! import's `ForeignImportSite::Item` index is rebased onto the spliced
//! list here too, and the import table is rebuilt in execution order so two
//! imports that land on one position still run in source order.

use crate::{ForeignImportSite, HirItem, ImportBinding};

/// Where an input of [`super::link_layout`] is spliced: into input
/// `parent`, before the first of its items whose source offset is at or
/// after `offset` (the source offset of the import statement that first
/// ran the input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub parent: usize,
    pub offset: u32,
}

/// Which linked-program position came from which input: one owner per
/// item and per import binding, in the linked order, and one per class
/// (classes stay in input order). Items `finalize` appends later have no
/// owner here; the driver attributes them to the entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkLayout {
    pub item_owner: Vec<usize>,
    pub import_owner: Vec<usize>,
    pub class_owner: Vec<usize>,
}

/// One input's items, import bindings and item origins, as the ordering
/// consumes them.
pub(super) struct Parts {
    pub(super) items: Vec<HirItem>,
    pub(super) imports: Vec<ImportBinding>,
    pub(super) origins: Vec<u32>,
    pub(super) placement: Option<Placement>,
}

/// One thing that happens at a position of an input's item list.
enum Event {
    Child(usize),
    Import(usize),
}

pub(super) struct Ordered {
    pub(super) items: Vec<HirItem>,
    pub(super) imports: Vec<ImportBinding>,
    pub(super) item_owner: Vec<usize>,
    pub(super) import_owner: Vec<usize>,
}

/// Splices every input into execution order.
pub(super) fn order(parts: Vec<Parts>) -> Ordered {
    let mut children: Vec<Vec<(u32, usize)>> = vec![Vec::new(); parts.len()];
    let mut roots = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        match part.placement {
            Some(placement) if placement.parent < parts.len() && placement.parent != index => {
                children[placement.parent].push((placement.offset, index));
            }
            _ => roots.push(index),
        }
    }
    let mut slots: Vec<Option<Parts>> = parts.into_iter().map(Some).collect();
    let mut out = Ordered {
        items: Vec::new(),
        imports: Vec::new(),
        item_owner: Vec::new(),
        import_owner: Vec::new(),
    };
    for root in roots {
        emit(root, &mut slots, &children, &mut out);
    }
    out
}

fn emit(
    index: usize,
    slots: &mut [Option<Parts>],
    children: &[Vec<(u32, usize)>],
    out: &mut Ordered,
) {
    let Some(part) = slots[index].take() else {
        // Already emitted (a placement cycle the driver never produces).
        return;
    };
    let len = part.items.len();
    let origin = |position: usize| part.origins.get(position).copied().unwrap_or(u32::MAX);
    // The position each child and each item-sited foreign import goes
    // before, with the source offset that orders events at one position.
    let mut events: Vec<(usize, u32, Event)> = Vec::new();
    for &(offset, child) in &children[index] {
        let position = (0..len)
            .find(|&position| origin(position) != u32::MAX && origin(position) >= offset)
            .unwrap_or(len);
        events.push((position, offset, Event::Child(child)));
    }
    let mut imports: Vec<Option<ImportBinding>> = part.imports.into_iter().map(Some).collect();
    for binding in 0..imports.len() {
        let sited = match &imports[binding] {
            Some(ImportBinding::Foreign {
                site: ForeignImportSite::Item(position),
                span,
                ..
            }) => Some((*position, span.start)),
            _ => None,
        };
        match sited {
            Some((position, start)) => {
                events.push((position.min(len), start, Event::Import(binding)));
            }
            None => {
                // Compile-time bindings and block-sited imports have no
                // position; they keep the input's own order, first.
                let binding = imports[binding].take().expect("each binding is taken once");
                out.imports.push(binding);
                out.import_owner.push(index);
            }
        }
    }
    events.sort_by_key(|(position, offset, _)| (*position, *offset));
    let mut events = events.into_iter().peekable();
    let mut items = part.items.into_iter();
    for position in 0..=len {
        while let Some((_, _, event)) = events.next_if(|(at, _, _)| *at == position) {
            match event {
                Event::Child(child) => emit(child, slots, children, out),
                Event::Import(binding) => {
                    let Some(ImportBinding::Foreign {
                        local_name,
                        module_path,
                        from,
                        span,
                        ..
                    }) = imports[binding].take()
                    else {
                        unreachable!("only item-sited foreign imports are events");
                    };
                    out.imports.push(ImportBinding::Foreign {
                        local_name,
                        module_path,
                        from,
                        site: ForeignImportSite::Item(out.items.len()),
                        span,
                    });
                    out.import_owner.push(index);
                }
            }
        }
        if let Some(item) = items.next() {
            out.items.push(item);
            out.item_owner.push(index);
        }
    }
}
