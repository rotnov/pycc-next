//! `bench/package_check.py` treats a downloaded archive as untrusted: a
//! member that is not a regular file or a directory (a symlink, a hard link,
//! a FIFO) is refused before anything is extracted, and the trees it hashes
//! and copies are walked without following links. A crafted archive with
//! each kind of link is rejected; a plain one is accepted.

use std::process::Command;

const SCRIPT: &str = r#"
import io, os, pathlib, sys, tarfile, tempfile
sys.path.insert(0, os.path.join(sys.argv[1], "bench"))
import package_check as pc

def archive(path, kind):
    with tarfile.open(path, "w:gz") as tar:
        data = b"x = 1\n"
        info = tarfile.TarInfo("pkg-1/src/mod.py")
        info.size = len(data)
        tar.addfile(info, io.BytesIO(data))
        if kind != "plain":
            info = tarfile.TarInfo("pkg-1/src/other.py")
            info.type = {"symlink": tarfile.SYMTYPE, "hardlink": tarfile.LNKTYPE,
                         "fifo": tarfile.FIFOTYPE}[kind]
            info.linkname = "/etc/passwd" if kind == "symlink" else "pkg-1/src/mod.py"
            tar.addfile(info)

pkg = pc.PACKAGES["tomli"]
with tempfile.TemporaryDirectory() as tmp:
    tmp = pathlib.Path(tmp)
    for kind in ("plain", "symlink", "hardlink", "fifo"):
        path = tmp / f"{kind}.tar.gz"
        archive(path, kind)
        with tarfile.open(path) as tar:
            try:
                pc.check_members(tar)
                print(kind, "accepted")
            except pc.UnsafeArchive as e:
                print(kind, "refused:", "is not a regular file" in str(e))
    # A symlink planted in an extracted tree is refused by the walk.
    tree = tmp / "tree"
    (tree / "src").mkdir(parents=True)
    (tree / "src" / "a.py").write_text("x = 1\n")
    print("tree", len(pc.regular_files(tree)))
    (tree / "src" / "b.py").symlink_to("/etc/passwd")
    try:
        pc.regular_files(tree)
        print("linked tree accepted")
    except pc.UnsafeArchive:
        print("linked tree refused")
    try:
        pc.regular_files(tree / "src" / "b.py")
        print("link accepted")
    except pc.UnsafeArchive:
        print("link refused")
"#;

#[test]
#[ignore = "requires python3 (PYCC_PYTHON, default python3)"]
fn a_crafted_archive_with_a_link_is_refused() {
    let python = std::env::var_os("PYCC_PYTHON").unwrap_or_else(|| "python3".into());
    let out = Command::new(python)
        .args(["-I", "-B", "-c", SCRIPT, env!("CARGO_MANIFEST_DIR")])
        .output()
        .expect("python");
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    assert!(out.status.success(), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        stdout,
        "plain accepted\nsymlink refused: True\nhardlink refused: True\nfifo refused: True\n\
         tree 1\nlinked tree refused\nlink refused\n"
    );
}
