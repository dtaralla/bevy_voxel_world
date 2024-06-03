use bevy::app::{App, Startup, Update};
use bevy::math::IVec3;
use bevy::prelude::{
    default, Camera3dBundle, Commands, Direction3d, Ray3d, Resource, Transform, Vec3,
};
use bevy::utils::HashMap;
use bevy::MinimalPlugins;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use noise::{HybridMulti, NoiseFn, Perlin};

use bevy_voxel_world::prelude::*;

#[derive(Resource, Clone, Default)]
struct MainWorld;

impl VoxelWorldConfig for MainWorld {
    fn spawning_distance(&self) -> u32 {
        25
    }

    fn voxel_lookup_delegate(&self) -> VoxelLookupDelegate {
        Box::new(move |_chunk_pos| get_voxel_fn())
    }
}

fn get_voxel_fn() -> Box<dyn FnMut(IVec3) -> WorldVoxel + Send + Sync> {
    // Set up some noise to use as the terrain height map
    let mut noise = HybridMulti::<Perlin>::new(1234);
    noise.octaves = 5;
    noise.frequency = 1.1;
    noise.lacunarity = 2.8;
    noise.persistence = 0.4;

    // We use this to cache the noise value for each y column so we only need
    // to calculate it once per x/z coordinate
    let mut cache = HashMap::<(i32, i32), f64>::new();

    // Then we return this boxed closure that captures the noise and the cache
    // This will get sent off to a separate thread for meshing by bevy_voxel_world
    Box::new(move |pos: IVec3| {
        // Sea level
        if pos.y < 1 {
            return WorldVoxel::Solid(3);
        }

        let [x, y, z] = pos.as_dvec3().to_array();

        // If y is less than the noise sample, we will set the voxel to solid
        let is_ground = y < match cache.get(&(pos.x, pos.z)) {
            Some(sample) => *sample,
            None => {
                let sample = noise.get([x / 1000.0, z / 1000.0]) * 50.0;
                cache.insert((pos.x, pos.z), sample);
                sample
            }
        };

        if is_ground {
            // Solid voxel of material type 0
            WorldVoxel::Solid(0)
        } else {
            WorldVoxel::Air
        }
    })
}

fn _test_setup_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, VoxelWorldPlugin::<MainWorld>::minimal()));
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((
            Camera3dBundle {
                transform: Transform::from_xyz(10.0, 10.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
                ..default()
            },
            VoxelWorldCamera::<MainWorld>::default(),
        ));
    });

    app
}

pub fn bench_raycast(c: &mut Criterion) {
    let mut group = c.benchmark_group("Raycast Diff");
    group.sample_size(200);

    let mut app = _test_setup_app();
    let mut app2 = _test_setup_app();
    for _ in 0..30 {
        app.update();
        app2.update();
    }

    let inputs = [
        Ray3d {
            origin: Vec3::new(10. * 32., 10. * 32., 10. * 32.),
            direction: -Direction3d::Y,
        },
        Ray3d {
            origin: Vec3::new(10. * 32., 10. * 32., 10. * 32.),
            direction: Direction3d::new(Vec3::new(-1., -1., -1.)).unwrap(),
        },
    ];

    app.add_systems(Update, move |voxel_world: VoxelWorld<MainWorld>| {
        let mut s = 0.;
        for r in inputs.iter() {
            let result = voxel_world.old_raycast(*r, &|(_pos, _vox)| true);
            s += result.map_or(0., |res| res.position.x);
        }
        black_box(s);
    });

    app2.add_systems(Update, move |voxel_world: VoxelWorld<MainWorld>| {
        let mut s = 0.;
        for r in inputs.iter() {
            let result = voxel_world.raycast(*r, &|(_pos, _vox)| true);
            s += result.map_or(0., |res| res.position.x);
        }
        black_box(s);
    });

    group.bench_function("Old", |b| {
        b.iter(|| {
            app.update();
        })
    });
    group.bench_function("New", |b| {
        b.iter(|| {
            app2.update();
        })
    });

    group.finish();
}

criterion_group!(line_traversal_benches, bench_raycast);
criterion_main!(line_traversal_benches);
