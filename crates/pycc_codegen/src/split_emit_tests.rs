use super::*;
use inkwell::memory_buffer::MemoryBuffer;

/// A program of `functions` internal functions of `adds` additions each,
/// sharing a mutable internal global and a private string constant, which
/// prints their results: large enough to split.
fn program_ir(functions: usize, adds: usize) -> String {
    let mut ir = String::from(
        "@counter = internal global i64 0\n\
         @fmt = private unnamed_addr constant [5 x i8] c\"%ld\\0A\\00\"\n\
         @table = internal constant [2 x i64] [i64 7, i64 11]\n\
         declare i32 @printf(ptr, ...)\n",
    );
    for f in 0..functions {
        ir.push_str(&format!(
            "define internal i64 @f{f}(i64 %x) {{\nentry:\n  %v0 = add i64 %x, {f}\n"
        ));
        for i in 1..adds {
            ir.push_str(&format!("  %v{i} = add i64 %v{}, {i}\n", i - 1));
        }
        ir.push_str(&format!(
            "  %c = load i64, ptr @counter\n  %c1 = add i64 %c, 1\n  store i64 %c1, ptr @counter\n\
             %t = load i64, ptr getelementptr (i64, ptr @table, i64 1)\n\
             %r = add i64 %v{}, %t\n  ret i64 %r\n}}\n",
            adds - 1
        ));
    }
    ir.push_str("define i32 @main() {\nentry:\n");
    for f in 0..functions {
        ir.push_str(&format!(
            "  %r{f} = call i64 @f{f}(i64 {f})\n  call i32 (ptr, ...) @printf(ptr @fmt, i64 %r{f})\n"
        ));
    }
    ir.push_str("  %n = load i64, ptr @counter\n  call i32 (ptr, ...) @printf(ptr @fmt, i64 %n)\n  ret i32 0\n}\n");
    ir
}

fn parse<'ctx>(context: &'ctx Context, ir: &str) -> Module<'ctx> {
    let text = format!("{ir}\0");
    let buffer = MemoryBuffer::create_from_memory_range_copy(text.as_bytes(), "test");
    context
        .create_module_from_ir(buffer)
        .expect("valid test IR")
}

#[test]
fn a_small_module_is_emitted_whole() {
    let context = Context::create();
    let module = parse(&context, &program_ir(2, 10));
    assert!(plan_for(&module, 4, "").is_none());
}

#[test]
fn one_thread_never_splits() {
    let context = Context::create();
    let module = parse(&context, &program_ir(6, 5_000));
    assert!(plan_for(&module, 1, "").is_none());
}

#[test]
fn a_large_module_is_balanced_over_the_partitions_with_its_locals_promoted() {
    let context = Context::create();
    let module = parse(&context, &program_ir(6, 5_000));
    let parts = plan_for(&module, 3, "").expect("30,000 instructions split");
    assert_eq!(parts.count, 3);
    let mut per_part = vec![0; parts.count];
    for part in parts.owner.values() {
        per_part[*part] += 1;
    }
    // Six equal functions and `main`, three partitions.
    assert!(per_part.iter().all(|n| (2..=3).contains(n)), "{per_part:?}");
    let f0 = module.get_function("__pycc_local.f0").expect("promoted");
    assert_eq!(f0.get_linkage(), inkwell::module::Linkage::External);
    assert!(module.get_global("__pycc_local.counter").is_some());
    assert!(
        module.get_global("__pycc_local.table").is_some(),
        "no unnamed_addr: shared"
    );
    assert!(
        module.get_global("fmt").is_some(),
        "an unnamed_addr constant is copied"
    );
}

#[test]
fn the_partitions_link_into_the_program_the_whole_module_is() {
    if cfg!(windows) {
        return;
    }
    let scratch = pycc_scratch::ScratchDir::new("split-emit").unwrap();
    let ir = program_ir(6, 5_000);
    let run = |objects: &[PathBuf], exe: &str| {
        let exe = scratch.join(exe);
        let status = std::process::Command::new("cc")
            .args(objects)
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap();
        assert!(status.success());
        let out = std::process::Command::new(&exe).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let whole = {
        let context = Context::create();
        let module = parse(&context, &ir);
        let path = scratch.join("whole.o");
        target_machine::target_machine_for(None, true)
            .unwrap()
            .write_to_file(&module, FileType::Object, &path)
            .unwrap();
        run(&[path], "whole")
    };
    let context = Context::create();
    let module = parse(&context, &ir);
    let parts = plan_for(&module, 3, "").unwrap();
    let objects = emit(&module, &parts, &scratch.join("main.o"), None, true).unwrap();
    assert_eq!(objects.len(), 3);
    assert_eq!(objects[0], scratch.join("main.o"));
    assert_eq!(objects[2], scratch.join("main.part2.o"));
    assert_eq!(run(&objects, "split"), whole);
    assert!(
        whole.ends_with("6\n"),
        "every function counted once: {whole}"
    );
}
