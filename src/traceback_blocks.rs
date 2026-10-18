//! The source block a native program prints under each traceback line of a
//! compiled frame (v0.8 accept 3): the statement's lines and, where
//! CPython draws them, its carets.
//!
//! CPython renders these at print time from the source file
//! (`traceback.StackSummary.format_frame_summary`). A native program has
//! no interpreter, so the build asks CPython once for the block of every
//! position the program can report, with the same function, and embeds
//! the text; the program prints a block only when its source file is
//! readable when it runs, as CPython prints a line only when it can read
//! it. With no interpreter at build time a traceback names each frame's
//! file, line and function only.

use pycc_hir::SrcPos;
use pycc_mir::{MirItem, MirModule, MirStmt};
use std::collections::{BTreeSet, HashMap};
use std::io::Write as _;
use std::process::{Command, Stdio};

/// Formats each request `[file, line, end_line, col, end_col]` (a negative
/// column is "none") as `format_frame_summary` does, and answers the text
/// after the `File` line of each.
const SCRIPT: &str = "import sys,json,traceback\n\
    out=[]\n\
    for f,l,el,c,ec in json.loads(sys.stdin.read()):\n\
    \x20 try:\n\
    \x20   s=traceback.StackSummary().format_frame_summary(traceback.FrameSummary(f,l,'f',end_lineno=el,colno=None if c<0 else c,end_colno=None if ec<0 else ec))\n\
    \x20   out.append(s.split('\\n',1)[1])\n\
    \x20 except Exception:\n\
    \x20   out.append('')\n\
    sys.stdout.write(json.dumps(out))\n";

/// The file names and source blocks a build's compiled frames carry: a
/// program's absolute paths, as CPython names a script's (and a native
/// one's blocks: an embedded one's interpreter renders its own), or for an
/// `--ext` module the paths it was built from (it names its frames' files
/// at import, next to its own `__file__`) and no blocks.
pub(crate) fn frame_sources(
    mir: &MirModule,
    kind: FrameSources,
) -> (Vec<String>, HashMap<SrcPos, String>) {
    let files = pycc_hir::positions::source_files();
    if kind == FrameSources::Module {
        return (files, HashMap::new());
    }
    let files: Vec<String> = files.iter().map(|file| absolute(file)).collect();
    let blocks = match kind {
        FrameSources::Native => source_blocks(mir, &files),
        _ => HashMap::new(),
    };
    (files, blocks)
}

/// What a build's frames are for ([`frame_sources`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameSources {
    /// A native program, which prints its own tracebacks.
    Native,
    /// A native program running in an embedded CPython.
    Embedded,
    /// An `--ext` module.
    Module,
}

/// `path` made absolute against the current directory with `.` and `..`
/// components resolved, as `os.path.abspath` does.
pub(crate) fn absolute(path: &str) -> String {
    let joined = match std::env::current_dir() {
        Ok(cwd) if !std::path::Path::new(path).is_absolute() => cwd.join(path),
        _ => std::path::PathBuf::from(path),
    };
    let mut out = std::path::PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

fn collect(
    body: &[MirStmt],
    sites: &[Vec<pycc_hir::positions::Site>],
    out: &mut BTreeSet<(u32, u32, u32, u32, u32)>,
) {
    for stmt in body {
        if let MirStmt::Pos(p) = stmt {
            out.insert((p.file, p.line, p.end_line, p.col, p.end_col));
            for (_, s) in sites.get(p.sites as usize).into_iter().flatten() {
                out.insert((s.file, s.line, s.end_line, s.col, s.end_col));
            }
        }
        for inner in stmt.bodies() {
            collect(inner, sites, out);
        }
    }
}

/// The source block of every position in `mir`, keyed by position, from
/// the interpreter `PYCC_PYTHON` names (default `python3`); empty when it
/// cannot run. `files` are the display paths by `SrcPos::file`.
fn source_blocks(mir: &MirModule, files: &[String]) -> HashMap<SrcPos, String> {
    let mut positions = BTreeSet::new();
    let sites = pycc_hir::positions::position_sites();
    for item in &mir.items {
        match item {
            MirItem::Function { body, .. } => collect(body, &sites, &mut positions),
            MirItem::TopLevelStmt(stmt) => {
                collect(std::slice::from_ref(stmt), &sites, &mut positions);
            }
            _ => {}
        }
    }
    let positions: Vec<_> = positions
        .into_iter()
        .filter(|&(file, ..)| (file as usize) < files.len())
        .collect();
    if positions.is_empty() {
        return HashMap::new();
    }
    let request: Vec<serde_json::Value> = positions
        .iter()
        .map(|&(file, line, end_line, col, end_col)| {
            let column = |c: u32| if c == u32::MAX { -1 } else { i64::from(c) };
            serde_json::json!([
                files[file as usize],
                line,
                end_line,
                column(col),
                column(end_col)
            ])
        })
        .collect();
    let python = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let Ok(mut child) = Command::new(python)
        .args(["-I", "-c", SCRIPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return HashMap::new();
    };
    let input = serde_json::to_vec(&request).unwrap_or_default();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&input);
    }
    let Ok(output) = child.wait_with_output() else {
        return HashMap::new();
    };
    let Ok(texts) = serde_json::from_slice::<Vec<String>>(&output.stdout) else {
        return HashMap::new();
    };
    positions
        .into_iter()
        .zip(texts)
        .filter(|(_, text)| !text.is_empty())
        .map(|((file, line, end_line, col, end_col), text)| {
            (
                SrcPos {
                    file,
                    line,
                    end_line,
                    col,
                    end_col,
                    sites: u32::MAX,
                },
                text,
            )
        })
        .collect()
}
