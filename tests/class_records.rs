//! Class records (v0.10, `pycc_codegen`'s `class_record`): every instance
//! points at one named descriptor global per class. A generic class's
//! specialization keeps a descriptor of its own, whose slot kinds are the
//! specialization's, so freeing an instance releases its `str` slots.

#[path = "stdlib_support/mod.rs"]
mod support;

#[test]
fn instances_of_a_generic_specialization_do_not_leak() {
    let source = support::fixture("class_record_generic_leaks.py");
    let dir = pycc_scratch::ScratchDir::new("class_record_generic_leaks").expect("scratch");
    let build = support::pycc()
        .arg("build")
        .arg(&source)
        .arg("-o")
        .arg(dir.join("class_record_generic_leaks"))
        .arg("--ext")
        .output()
        .expect("pycc spawns");
    assert!(build.status.success(), "{}", support::rendered(&build));
    let driver = "import ctypes\n\
                  import class_record_generic_leaks as m\n\
                  lib = ctypes.CDLL(m.__file__)\n\
                  strs = lib.pycc_rt_str_live_objects\n\
                  strs.restype = ctypes.c_int64\n\
                  objs = lib.pycc_rt_instance_live_objects\n\
                  objs.restype = ctypes.c_int64\n\
                  print(m.churn(5))\n\
                  before = (strs(), objs())\n\
                  for _ in range(200):\n    \
                  m.churn(5)\n\
                  after = (strs(), objs())\n\
                  assert after == before, 'leaked: %r -> %r' % (before, after)\n\
                  print('ok')\n";
    let run = std::process::Command::new(support::python())
        .args(["-B", "-c", driver])
        .current_dir(&*dir)
        .output()
        .expect("python3 runs");
    assert_eq!(
        pycc_scratch::lf(&*String::from_utf8_lossy(&run.stdout)).replace("\r\n", "\n"),
        "23\nok\n",
        "{}",
        pycc_scratch::lf(&*String::from_utf8_lossy(&run.stderr))
    );
}
