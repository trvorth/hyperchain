use criterion::{criterion_group, criterion_main, Criterion};
use my_blockchain::reliable_hashing_algorithm;

fn benchmark_hash(c: &mut Criterion) {
    let input = b"test_input";
    c.bench_function("reliable_hashing_algorithm", |b| {
        b.iter(|| reliable_hashing_algorithm(input))
    });
}

criterion_group!(benches, benchmark_hash);
criterion_main!(benches);

