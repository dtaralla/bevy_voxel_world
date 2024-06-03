///
/// VoxelWorld
/// This module implements most of the public API for bevy_voxel_world.
///
use std::marker::PhantomData;
use std::sync::Arc;

use bevy::{ecs::system::SystemParam, math::bounding::RayCast3d, prelude::*};

use crate::voxel::VoxelAabb;
use crate::{
    chunk_map::ChunkMap,
    configuration::VoxelWorldConfig,
    traversal_alg::voxel_line_traversal,
    voxel::WorldVoxel,
    voxel_world_internal::{get_chunk_voxel_position, ModifiedVoxels, VoxelWriteBuffer},
};

/// This component is used to mark the Camera that bevy_voxel_world should use to determine
/// which chunks to spawn and despawn.
#[derive(Component)]
pub struct VoxelWorldCamera<C> {
    _marker: PhantomData<C>,
}

impl<C> Default for VoxelWorldCamera<C> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

#[derive(Event)]
pub struct ChunkEvent<C> {
    pub chunk_key: IVec3,
    pub entity: Entity,
    _marker: PhantomData<C>,
}

impl<C> ChunkEvent<C> {
    pub fn new(chunk_key: IVec3, entity: Entity) -> Self {
        Self {
            chunk_key,
            entity,
            _marker: PhantomData,
        }
    }

    pub fn clone(&self) -> Self {
        Self {
            chunk_key: self.chunk_key,
            entity: self.entity,
            _marker: PhantomData,
        }
    }
}

/// Fired when a chunk is about to be despawned.
pub type ChunkWillDespawn<C> = ChunkEvent<C>;

/// Fired when a chunk is about to be spawned.
pub type ChunkWillSpawn<C> = ChunkEvent<C>;

/// Fired when a chunk is about to be remeshed.
pub type ChunkWillRemesh<C> = ChunkEvent<C>;

pub trait FilterFn {
    fn call(&self, input: (Vec3, WorldVoxel)) -> bool;
}

impl<F: Fn((Vec3, WorldVoxel)) -> bool> FilterFn for F {
    fn call(&self, input: (Vec3, WorldVoxel)) -> bool {
        self(input)
    }
}

pub type RaycastFn = dyn Fn(Ray3d, &dyn FilterFn) -> Option<VoxelRaycastResult> + Send + Sync;

#[derive(Default, Debug, PartialEq, Clone)]
pub struct VoxelRaycastResult {
    pub position: Vec3,
    pub normal: Option<Vec3>,
    pub voxel: WorldVoxel,
}

impl VoxelRaycastResult {
    /// Get the voxel position of the raycast result
    pub fn voxel_pos(&self) -> IVec3 {
        self.position.floor().as_ivec3()
    }

    /// Get the face normal of the ray hit
    pub fn voxel_normal(&self) -> Option<IVec3> {
        self.normal.map(|n| n.floor().as_ivec3())
    }
}

/// Grants access to the VoxelWorld in systems
#[derive(SystemParam)]
pub struct VoxelWorld<'w, C: VoxelWorldConfig> {
    chunk_map: Res<'w, ChunkMap<C>>,
    modified_voxels: Res<'w, ModifiedVoxels<C>>,
    voxel_write_buffer: ResMut<'w, VoxelWriteBuffer<C>>,
    #[allow(unused)]
    configuration: Res<'w, C>,
}

impl<'w, C: VoxelWorldConfig> VoxelWorld<'w, C> {
    /// Get the voxel at the given position. The voxel will be WorldVoxel::Unset if there is no voxel at that position
    pub fn get_voxel(&self, position: IVec3) -> WorldVoxel {
        self.get_voxel_fn()(position)
    }

    /// Set the voxel at the given position. This will create a new chunk if one does not exist at
    /// the given position.
    pub fn set_voxel(&mut self, position: IVec3, voxel: WorldVoxel) {
        self.voxel_write_buffer.push((position, voxel));
    }

    /// Get a sendable closure that can be used to get the voxel at the given position
    /// This is useful for spawning tasks that need to access the voxel world
    pub fn get_voxel_fn(&self) -> Arc<dyn Fn(IVec3) -> WorldVoxel + Send + Sync> {
        let chunk_map = self.chunk_map.get_map();
        let write_buffer = self.voxel_write_buffer.clone();
        let modified_voxels = self.modified_voxels.clone();

        Arc::new(move |position| {
            let (chunk_pos, vox_pos) = get_chunk_voxel_position(position);

            if let Some(voxel) = write_buffer
                .iter()
                .find(|(pos, _)| *pos == position)
                .map(|(_, voxel)| *voxel)
            {
                return voxel;
            }

            {
                if let Some(voxel) = modified_voxels.get_voxel(&position) {
                    return voxel;
                }
            }

            let chunk_opt = {
                let chun_map_read = chunk_map.read().unwrap();
                chun_map_read.get(&chunk_pos).cloned()
            };

            if let Some(chunk_data) = chunk_opt {
                chunk_data.get_voxel(vox_pos)
            } else {
                WorldVoxel::Unset
            }
        })
    }

    /// Get the closes surface voxel to the given position
    /// Returns None if there is no surface voxel at or below the given position
    pub fn get_closest_surface_voxel(&self, position: IVec3) -> Option<(IVec3, WorldVoxel)> {
        let get_voxel = self.get_voxel_fn();
        let mut current_pos = position;
        let current_voxel = get_voxel(current_pos);

        let is_surface = |pos: IVec3| {
            let above = pos + IVec3::Y;
            (get_voxel(pos) != WorldVoxel::Unset && get_voxel(pos) != WorldVoxel::Air)
                && (get_voxel(above) == WorldVoxel::Unset || get_voxel(above) == WorldVoxel::Air)
        };

        if current_voxel == WorldVoxel::Unset || current_voxel == WorldVoxel::Air {
            while !is_surface(current_pos) {
                current_pos -= IVec3::Y;
                if current_pos.y < -256 {
                    return None;
                }
            }

            return Some((current_pos, get_voxel(current_pos)));
        }

        None
    }

    /// Get a randowm surface voxel within the given radius of the given position
    /// Returns None if no surface voxel was found within the given radius
    pub fn get_random_surface_voxel(
        &self,
        position: IVec3,
        radius: u32,
    ) -> Option<(IVec3, WorldVoxel)> {
        let mut tries = 0;

        while tries < 100 {
            tries += 1;

            let r = radius as f32;
            let x = rand::random::<f32>() * r * 2.0 - r;
            let y = rand::random::<f32>() * r * 2.0 - r;
            let z = rand::random::<f32>() * r * 2.0 - r;

            if y < 0.0 {
                continue;
            }

            let d = x * x + y * y + z * z;
            if d > r * r {
                continue;
            }

            let pos = position + IVec3::new(x as i32, y as i32, z as i32);
            if let Some(result) = self.get_closest_surface_voxel(pos) {
                return Some(result);
            }
        }

        None
    }

    /// Get first surface voxel at the given Vec2 position
    pub fn get_surface_voxel_at_2d_pos(&self, pos_2d: Vec2) -> Option<(IVec3, WorldVoxel)> {
        self.get_closest_surface_voxel(IVec3 {
            x: pos_2d.x.floor() as i32,
            y: 256,
            z: pos_2d.y.floor() as i32,
        })
    }

    /// Get the first solid voxel intersecting with the given ray.
    /// The `filter` function can be used to filter out voxels that should not be considered for the raycast.
    ///
    /// Returns a `VoxelRaycastResult` with position, normal and voxel info. The position is given in world space.
    /// Returns `None` if no voxel was intersected
    ///
    /// Note: The method used for raycasting here is not 100% accurate. It is possible for the ray to miss a voxel
    /// if the ray is very close to the edge. This is because the raycast is done in steps of 0.01 units.
    /// If you need 100% accuracy, it may be better to cast against the mesh instead, using something like `bevy_mod_raycast`
    /// or some physics plugin.
    ///
    /// # Example
    /// ```
    /// use bevy::prelude::*;
    /// use bevy_voxel_world::prelude::*;
    ///
    /// fn do_raycast(
    ///     voxel_world: VoxelWorld<DefaultWorld>,
    ///     camera_info: Query<(&Camera, &GlobalTransform), With<VoxelWorldCamera<DefaultWorld>>>,
    ///     mut cursor_evr: EventReader<CursorMoved>,
    /// ) {
    ///     for ev in cursor_evr.read() {
    ///         // Get a ray from the cursor position into the world
    ///         let (camera, cam_gtf) = camera_info.single();
    ///         let Some(ray) = camera.viewport_to_world(cam_gtf, ev.position) else {
    ///            return;
    ///         };
    ///
    ///         if let Some(result) = voxel_world.raycast(ray, &|(_pos, _vox)| true) {
    ///             println!("vox_pos: {:?}, normal: {:?}, vox: {:?}", result.position, result.normal, result.voxel);
    ///         }
    ///     }
    /// }
    /// ```
    pub fn raycast(
        &self,
        ray: Ray3d,
        filter: &impl Fn((Vec3, WorldVoxel)) -> bool,
    ) -> Option<VoxelRaycastResult> {
        let raycast_fn = self.raycast_fn();
        raycast_fn(ray, filter)
    }

    pub fn old_raycast(
        &self,
        ray: Ray3d,
        filter: &impl Fn((Vec3, WorldVoxel)) -> bool,
    ) -> Option<VoxelRaycastResult> {
        let raycast_fn = self.old_raycast_fn();
        raycast_fn(ray, filter)
    }

    pub fn old_raycast_fn(&self) -> Arc<RaycastFn> {
        let chunk_map = self.chunk_map.get_map();
        let spawning_distance = self.configuration.spawning_distance() as i32;
        let get_voxel = self.get_voxel_fn();

        fn get_hit_normal(vox_pos: IVec3, ray: Ray3d) -> Option<Vec3> {
            let voxel_aabb = bevy::render::primitives::Aabb::from_min_max(
                vox_pos.as_vec3(),
                vox_pos.as_vec3() + Vec3::ONE,
            );

            let (_, normal) = voxel_aabb.ray_intersection(ray)?;

            Some(normal)
        }
        const STEP_SIZE: f32 = 0.01;

        Arc::new(move |ray, filter| {
            let chunk_map_read_lock = chunk_map.read().unwrap();
            let mut current = ray.origin;
            let mut t = 0.0;

            while t < (spawning_distance * crate::chunk::CHUNK_SIZE_I) as f32 {
                let chunk_pos = (current / crate::chunk::CHUNK_SIZE_F).floor().as_ivec3();

                if let Some(chunk_data) = ChunkMap::<C>::get(&chunk_pos, &chunk_map_read_lock) {
                    if !chunk_data.is_empty {
                        let mut voxel = WorldVoxel::Unset;
                        while voxel == WorldVoxel::Unset && chunk_data.encloses_point(current) {
                            let mut voxel_pos = current.floor().as_ivec3();
                            voxel = get_voxel(voxel_pos);
                            if voxel.is_solid() {
                                let mut normal = get_hit_normal(voxel_pos, ray).unwrap();

                                let mut adjacent_vox = get_voxel(voxel_pos + normal.as_ivec3());

                                // When we get here we have an approximate hit position and normal,
                                // so we refine until the position adjacent to the normal is empty.
                                let mut steps = 0;
                                while adjacent_vox.is_solid() && steps < 3 {
                                    steps += 1;
                                    voxel = adjacent_vox;
                                    voxel_pos += normal.as_ivec3();
                                    normal = get_hit_normal(voxel_pos, ray).unwrap_or(normal);
                                    adjacent_vox = get_voxel(voxel_pos + normal.as_ivec3());
                                }

                                if filter.call((voxel_pos.as_vec3(), voxel)) {
                                    return Some(VoxelRaycastResult {
                                        position: voxel_pos.as_vec3(),
                                        normal: Some(normal),
                                        voxel,
                                    });
                                }
                            }
                            t += STEP_SIZE;
                            current = ray.origin + ray.direction * t;
                        }
                    }
                }

                t += STEP_SIZE;
                current = ray.origin + ray.direction * t;
            }
            None
        })
    }

    /// Get a sendable closure that can be used to raycast into the voxel world
    pub fn raycast_fn(&self) -> Arc<RaycastFn> {
        let chunk_map = self.chunk_map.get_map();
        let get_voxel = self.get_voxel_fn();

        Arc::new(move |ray, filter| {
            let p = ray.origin;
            let d = *ray.direction;

            let loaded_aabb = ChunkMap::<C>::get_world_bounds(&chunk_map.read().unwrap());
            let trace_start = if p.cmplt(loaded_aabb.min).any() || p.cmpgt(loaded_aabb.max).any() {
                if let Some(trace_start_t) =
                    RayCast3d::from_ray(ray, f32::MAX).aabb_intersection_at(&loaded_aabb)
                {
                    ray.get_point(trace_start_t)
                } else {
                    return None;
                }
            } else {
                p
            };

            // To find where we get out of the loaded cuboid, we can intersect from a point
            // guaranteed to be on the other side of the cube and in the opposite direction
            // of the ray.
            let trace_end_orig =
                trace_start + d * loaded_aabb.min.distance_squared(loaded_aabb.max);
            let trace_end_t = RayCast3d::new(trace_end_orig, -ray.direction, f32::MAX)
                .aabb_intersection_at(&loaded_aabb)
                .unwrap();
            let trace_end = Ray3d::new(trace_end_orig, -d).get_point(trace_end_t);

            let mut raycast_result = None;
            voxel_line_traversal(trace_start, trace_end, |voxel_coords, _time, face| {
                let voxel = get_voxel(voxel_coords);

                if !voxel.is_unset() && filter.call((voxel_coords.as_vec3(), voxel)) {
                    if voxel.is_solid() {
                        raycast_result = Some(VoxelRaycastResult {
                            position: voxel_coords.as_vec3(),
                            normal: face.try_into().ok(),
                            voxel,
                        });

                        // Found solid voxel - stop traversing
                        false
                    } else {
                        // Voxel is not solid - continue traversing
                        true
                    }
                } else {
                    // Ignoring this voxel bc of filter - continue traversing
                    true
                }
            });

            raycast_result
        })
    }
}
