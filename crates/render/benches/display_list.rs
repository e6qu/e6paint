//! Display-list build cost over the torture board (DESIGN §10): the budget is the whole
//! 16 ms frame, so a full-scene rebuild must land well under it. This measures the pure
//! CPU encoding; the GPU pass is measured on-device via the HUD.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use e6_render::build_display_list;
use e6_scene::{Camera, Size, torture_board};
use vello::Scene;

fn bench_display_list(c: &mut Criterion) {
    let elements = torture_board();
    let camera = Camera::default();
    let viewport = Size::new(1600.0, 900.0);

    let mut group = c.benchmark_group("display_list");
    group.throughput(Throughput::Elements(elements.len() as u64));
    group.bench_function("torture_board_10k", |bench| {
        bench.iter_batched(
            Scene::new,
            |mut scene| {
                let stats =
                    build_display_list(&mut scene, &camera, viewport, black_box(&elements), false);
                black_box(stats)
            },
            criterion::BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion::criterion_group!(benches, bench_display_list);
criterion::criterion_main!(benches);
