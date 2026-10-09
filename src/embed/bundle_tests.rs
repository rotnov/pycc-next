//! The stdlib walk the copy and the Windows import scan share
//! ([`kept_entries`]), and the copy built on it.

use super::*;
use pycc_scratch::ScratchDir;

fn write(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, b"x").expect("write");
}

/// The root comes first, each directory precedes its contents, a skipped
/// name is never entered, and an empty directory is kept.
#[test]
fn the_walk_keeps_directories_before_their_contents_and_empty_ones() {
    let dir = ScratchDir::new("embed_kept_entries").expect("scratch");
    let from = dir.join("from");
    write(&from.join("json").join("decoder.py"));
    write(&from.join("test").join("t.py"));
    std::fs::create_dir_all(from.join("empty")).expect("mkdir");
    let kept = kept_entries(&from, layout::skip_in_stdlib_copy).expect("walk");
    assert_eq!(kept.first(), Some(&Kept::Dir(PathBuf::new())));
    let json = Kept::Dir(PathBuf::from("json"));
    let decoder = Kept::File(Path::new("json").join("decoder.py"));
    let at = |entry: &Kept| kept.iter().position(|kept| kept == entry);
    assert!(
        at(&json) < at(&decoder) && at(&decoder).is_some(),
        "{kept:?}"
    );
    assert!(
        kept.contains(&Kept::Dir(PathBuf::from("empty"))),
        "{kept:?}"
    );
    assert_eq!(kept.len(), 4, "`test` is skipped: {kept:?}");
}

/// The copy creates every kept directory, an empty one included, and
/// copies every kept file's bytes.
#[test]
fn the_copy_creates_an_empty_kept_directory() {
    let dir = ScratchDir::new("embed_copy_stdlib").expect("scratch");
    let from = dir.join("from");
    write(&from.join("json").join("decoder.py"));
    std::fs::create_dir_all(from.join("empty")).expect("mkdir");
    let to = dir.join("to");
    copy_stdlib(&from, &to, layout::skip_in_stdlib_copy).expect("copy");
    assert!(to.join("empty").is_dir());
    assert_eq!(
        std::fs::read(to.join("json").join("decoder.py")).expect("read"),
        b"x"
    );
}
