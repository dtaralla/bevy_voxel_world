use std::hash::{BuildHasher, Hash};

use bevy::math::IVec3;
use bevy::utils::HashMap;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::prelude::SliceRandom;
use rand::SeedableRng;

#[derive(Hash, PartialEq, Eq, Copy, Clone)]
struct Coord3d(u32);

impl Coord3d {
    const X_BITS: u8 = 13;
    const Y_BITS: u8 = 13;
    const Z_BITS: u8 = 6;

    const X_BIAS: i32 = 1 << (Self::X_BITS - 1);
    const Y_BIAS: i32 = 1 << (Self::Y_BITS - 1);
    const Z_BIAS: i32 = 1 << (Self::Z_BITS - 1);

    const X_SHIFT: u8 = 0;
    const Y_SHIFT: u8 = Self::X_BITS;
    const Z_SHIFT: u8 = Self::X_BITS + Self::Y_BITS;

    pub fn from_coords(x: i32, y: i32, z: i32) -> Coord3d {
        let x: u32 = ((x + Self::X_BIAS) as u32) << Self::X_SHIFT;
        let y: u32 = ((y + Self::Y_BIAS) as u32) << Self::Y_SHIFT;
        let z: u32 = ((z + Self::Z_BIAS) as u32) << Self::Z_SHIFT;
        Self(z | y | x)
    }
}

trait FromXYZ {
    fn new(x: i32, y: i32, z: i32) -> Self;
}

impl FromXYZ for Coord3d {
    fn new(x: i32, y: i32, z: i32) -> Self {
        Coord3d::from_coords(x, y, z)
    }
}

impl FromXYZ for IVec3 {
    fn new(x: i32, y: i32, z: i32) -> Self {
        IVec3::new(x, y, z)
    }
}

const XY_LOW: i32 = -100;
const XY_UP: i32 = 100;
const Z_LOW: i32 = -10;
const Z_UP: i32 = 10;
const NUM_ELEMS: usize = ((XY_UP - XY_LOW) * (XY_UP - XY_LOW) * (Z_UP - Z_LOW)) as usize;

fn gen_coords<T: FromXYZ>() -> Vec<T> {
    let mut coords = Vec::<T>::with_capacity(NUM_ELEMS);
    for x in XY_LOW..XY_UP {
        for y in XY_LOW..XY_UP {
            for z in Z_LOW..Z_UP {
                coords.push(T::new(x, y, z));
            }
        }
    }

    coords
}

fn hashes<T: FromXYZ + Hash, B: BuildHasher>(coords: &[T], hasher_builder: &B) {
    for c in coords {
        black_box(hasher_builder.hash_one(c));
    }
}

fn inserts<T: FromXYZ + Hash + Eq + Copy>(coords: &[T]) -> HashMap<T, i32> {
    let mut hmap = HashMap::with_capacity(coords.len());

    for c in coords.iter() {
        hmap.insert(*c, 0);
    }

    hmap
}

fn reads<T: FromXYZ + Hash + Eq, V>(coords: &[T], hmap: &HashMap<T, V>) {
    for c in coords {
        black_box(hmap.get(c));
    }
}

pub fn bench_hashes(c: &mut Criterion) {
    let coords_i = gen_coords::<IVec3>();
    let coords_c = gen_coords::<Coord3d>();

    let hasher_builder = HashMap::<u32, u32>::default().hasher().clone();

    let mut group = c.benchmark_group("Hashes");
    group.sample_size(3000);
    group.bench_function("IVec3", |b| b.iter(|| hashes(&coords_i, &hasher_builder)));
    group.bench_function("Coord3d", |b| b.iter(|| hashes(&coords_c, &hasher_builder)));
    group.finish();
}

pub fn bench_inserts(c: &mut Criterion) {
    let coords_i = gen_coords::<IVec3>();
    let coords_c = gen_coords::<Coord3d>();

    // let seed = rand::random::<u64>();
    let seed = 4242;

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut random_i = coords_i.clone();
    random_i.shuffle(&mut rng);

    rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut random_c = coords_c.clone();
    random_c.shuffle(&mut rng);

    let mut group = c.benchmark_group("Inserts");
    group.sample_size(350);
    group.bench_function("IVec3 Ordered", |b| b.iter(|| inserts(&coords_i)));
    group.bench_function("IVec3 Random", |b| b.iter(|| inserts(&random_i)));
    group.bench_function("Coord3d Ordered", |b| b.iter(|| inserts(&coords_c)));
    group.bench_function("Coord3d Random", |b| b.iter(|| inserts(&random_c)));
    group.finish();
}

pub fn bench_reads(c: &mut Criterion) {
    let coords_i = gen_coords::<IVec3>();
    let coords_c = gen_coords::<Coord3d>();

    let hmap_i = inserts(&coords_i);
    let hmap_c = inserts(&coords_c);

    // let seed = rand::random::<u64>();
    let seed = 4242;

    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut random_i = coords_i.clone();
    random_i.shuffle(&mut rng);

    rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut random_c = coords_c.clone();
    random_c.shuffle(&mut rng);

    let mut group = c.benchmark_group("Reads");
    group.sample_size(350);
    group.bench_function("IVec3 Ordered", |b| b.iter(|| reads(&coords_i, &hmap_i)));
    group.bench_function("IVec3 Random", |b| b.iter(|| reads(&random_i, &hmap_i)));
    group.bench_function("Coord3d Ordered", |b| b.iter(|| reads(&coords_c, &hmap_c)));
    group.bench_function("Coord3d Random", |b| b.iter(|| reads(&random_c, &hmap_c)));
    group.finish();
}

criterion_group!(coord_hash_benches, bench_hashes, bench_inserts, bench_reads);
criterion_main!(coord_hash_benches);
