use criterion::{Criterion, criterion_group, criterion_main};

const FIXTURE: &str = r#"
def fib(n: int) -> int:
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)

def main() -> None:
    x = 0
    for i in range(10):
        x = x + fib(i)
    print(x)

main()
"#;

fn bench_check(c: &mut Criterion) {
    c.bench_function("pycc_check_frontend_fixture", |b| {
        b.iter(|| {
            let module = pycc_parser::parse(FIXTURE).unwrap();
            let hir = pycc_hir::lower_checked(&module).unwrap();
            pycc_types::check(&hir).unwrap();
        });
    });
}

criterion_group!(benches, bench_check);
criterion_main!(benches);
