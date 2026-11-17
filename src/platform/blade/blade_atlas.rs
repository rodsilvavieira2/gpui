use crate::{
    AtlasKey, AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, DevicePixels, PlatformAtlas,
    Point, Size, platform::AtlasTextureList,
};
use anyhow::Result;
use blade_graphics as gpu;
use blade_util::{BufferBelt, BufferBeltDescriptor};
use collections::FxHashMap;
use etagere::BucketedAtlasAllocator;
use parking_lot::Mutex;
use std::{borrow::Cow, ops, sync::Arc};

pub(crate) struct BladeAtlas(Mutex<BladeAtlasState>);

struct PendingUpload {
    id: AtlasTextureId,
    bounds: Bounds<DevicePixels>,
    data: gpu::BufferPiece,
}

struct BladeAtlasState {
    gpu: Arc<gpu::Context>,
    upload_belt: BufferBelt,
    storage: BladeAtlasStorage,
    tiles_by_key: FxHashMap<AtlasKey, AtlasTile>,
    initializations: Vec<AtlasTextureId>,
    uploads: Vec<PendingUpload>,
}

#[cfg(gles)]
unsafe impl Send for BladeAtlasState {}

impl BladeAtlasState {
    fn destroy(&mut self) {
        self.storage.destroy(&self.gpu);
        self.upload_belt.destroy(&self.gpu);
    }
}

pub struct BladeTextureInfo {
    pub raw_view: gpu::TextureView,
}

impl BladeAtlas {
    pub(crate) fn new(gpu: &Arc<gpu::Context>) -> Self {
        BladeAtlas(Mutex::new(BladeAtlasState {
            gpu: Arc::clone(gpu),
            upload_belt: BufferBelt::new(BufferBeltDescriptor {
                memory: gpu::Memory::Upload,
                min_chunk_size: 0x10000,
                alignment: 64, // Vulkan `optimalBufferCopyOffsetAlignment` on Intel XE
            }),
            storage: BladeAtlasStorage::default(),
            tiles_by_key: Default::default(),
            initializations: Vec::new(),
            uploads: Vec::new(),
        }))
    }

    pub(crate) fn destroy(&self) {
        self.0.lock().destroy();
    }

    pub fn before_frame(&self, gpu_encoder: &mut gpu::CommandEncoder) {
        let mut lock = self.0.lock();
        lock.flush(gpu_encoder);
    }

    pub fn after_frame(&self, sync_point: &gpu::SyncPoint) {
        let mut lock = self.0.lock();
        lock.upload_belt.flush(sync_point);
    }

    /// Returns the GPU view of `id`, or `None` when that atlas texture was
    /// already destroyed by [`PlatformAtlas::remove`] (an image dropped between
    /// paint and draw leaves primitives pointing at a freed texture).
    pub fn get_texture_info(&self, id: AtlasTextureId) -> Option<BladeTextureInfo> {
        let lock = self.0.lock();
        let texture = lock.storage.get(id)?;
        Some(BladeTextureInfo {
            raw_view: texture.raw_view,
        })
    }
}

impl PlatformAtlas for BladeAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        let mut lock = self.0.lock();
        if let Some(tile) = lock.tiles_by_key.get(key) {
            Ok(Some(tile.clone()))
        } else {
            profiling::scope!("new tile");
            let Some((size, bytes)) = build()? else {
                return Ok(None);
            };
            let tile = lock.allocate(size, key.texture_kind());
            lock.upload_texture(tile.texture_id, tile.bounds, &bytes);
            lock.tiles_by_key.insert(key.clone(), tile.clone());
            Ok(Some(tile))
        }
    }

    fn remove(&self, key: &AtlasKey) {
        let mut lock = self.0.lock();

        let Some(id) = lock.tiles_by_key.remove(key).map(|tile| tile.texture_id) else {
            return;
        };

        let Some(texture_slot) = lock.storage[id.kind].textures.get_mut(id.index as usize) else {
            return;
        };

        let Some(mut texture) = texture_slot.take() else {
            return;
        };

        texture.decrement_ref_count();
        if texture.is_unreferenced() {
            lock.storage[id.kind]
                .free_list
                .push(texture.id.index as usize);
            texture.destroy(&lock.gpu);
            // This texture may still be queued for initialization or upload in
            // `flush`, which runs at the start of the next frame. Drop that
            // work, otherwise `flush` would index a slot that no longer exists.
            lock.discard_pending(texture.id);
        } else {
            *texture_slot = Some(texture);
        }
    }
}

impl BladeAtlasState {
    fn allocate(&mut self, size: Size<DevicePixels>, texture_kind: AtlasTextureKind) -> AtlasTile {
        {
            let textures = &mut self.storage[texture_kind];

            if let Some(tile) = textures
                .iter_mut()
                .rev()
                .find_map(|texture| texture.allocate(size))
            {
                return tile;
            }
        }

        let texture = self.push_texture(size, texture_kind);
        texture.allocate(size).unwrap()
    }

    fn push_texture(
        &mut self,
        min_size: Size<DevicePixels>,
        kind: AtlasTextureKind,
    ) -> &mut BladeAtlasTexture {
        const DEFAULT_ATLAS_SIZE: Size<DevicePixels> = Size {
            width: DevicePixels(1024),
            height: DevicePixels(1024),
        };

        let size = min_size.max(&DEFAULT_ATLAS_SIZE);
        let format;
        let usage;
        match kind {
            AtlasTextureKind::Monochrome => {
                format = gpu::TextureFormat::R8Unorm;
                usage = gpu::TextureUsage::COPY | gpu::TextureUsage::RESOURCE;
            }
            AtlasTextureKind::Polychrome => {
                format = gpu::TextureFormat::Bgra8Unorm;
                usage = gpu::TextureUsage::COPY | gpu::TextureUsage::RESOURCE;
            }
        }

        let raw = self.gpu.create_texture(gpu::TextureDesc {
            name: "atlas",
            format,
            size: gpu::Extent {
                width: size.width.into(),
                height: size.height.into(),
                depth: 1,
            },
            array_layer_count: 1,
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::TextureDimension::D2,
            usage,
            external: None,
        });
        let raw_view = self.gpu.create_texture_view(
            raw,
            gpu::TextureViewDesc {
                name: "",
                format,
                dimension: gpu::ViewDimension::D2,
                subresources: &Default::default(),
            },
        );

        let texture_list = &mut self.storage[kind];
        let index = texture_list.free_list.pop();

        let atlas_texture = BladeAtlasTexture {
            id: AtlasTextureId {
                index: index.unwrap_or(texture_list.textures.len()) as u32,
                kind,
            },
            allocator: etagere::BucketedAtlasAllocator::new(size.into()),
            format,
            raw,
            raw_view,
            live_atlas_keys: 0,
        };

        self.initializations.push(atlas_texture.id);

        if let Some(ix) = index {
            texture_list.textures[ix] = Some(atlas_texture);
            texture_list.textures.get_mut(ix).unwrap().as_mut().unwrap()
        } else {
            texture_list.textures.push(Some(atlas_texture));
            texture_list.textures.last_mut().unwrap().as_mut().unwrap()
        }
    }

    fn upload_texture(&mut self, id: AtlasTextureId, bounds: Bounds<DevicePixels>, bytes: &[u8]) {
        let data = self.upload_belt.alloc_bytes(bytes, &self.gpu);
        self.uploads.push(PendingUpload { id, bounds, data });
    }

    /// Drops queued `flush` work that references a texture that no longer exists.
    fn discard_pending(&mut self, id: AtlasTextureId) {
        retain_pending(&mut self.initializations, |queued| *queued, id);
        retain_pending(&mut self.uploads, |upload| upload.id, id);
    }

    fn flush_initializations(&mut self, encoder: &mut gpu::CommandEncoder) {
        for id in self.initializations.drain(..) {
            // The texture can be gone: an image released between frames destroys
            // its atlas texture while this queue still holds its id.
            if let Some(texture) = self.storage.get(id) {
                encoder.init_texture(texture.raw);
            }
        }
    }

    fn flush(&mut self, encoder: &mut gpu::CommandEncoder) {
        self.flush_initializations(encoder);

        let mut transfers = encoder.transfer("atlas");
        for upload in self.uploads.drain(..) {
            let Some(texture) = self.storage.get(upload.id) else {
                continue;
            };
            transfers.copy_buffer_to_texture(
                upload.data,
                upload.bounds.size.width.to_bytes(texture.bytes_per_pixel()),
                gpu::TexturePiece {
                    texture: texture.raw,
                    mip_level: 0,
                    array_layer: 0,
                    origin: [
                        upload.bounds.origin.x.into(),
                        upload.bounds.origin.y.into(),
                        0,
                    ],
                },
                gpu::Extent {
                    width: upload.bounds.size.width.into(),
                    height: upload.bounds.size.height.into(),
                    depth: 1,
                },
            );
        }
    }
}

/// Keeps every queued entry whose texture id is still alive, preserving order.
fn retain_pending<T>(
    pending: &mut Vec<T>,
    id_of: impl Fn(&T) -> AtlasTextureId,
    discarded: AtlasTextureId,
) {
    pending.retain(|entry| id_of(entry) != discarded);
}

#[derive(Default)]
struct BladeAtlasStorage {
    monochrome_textures: AtlasTextureList<BladeAtlasTexture>,
    polychrome_textures: AtlasTextureList<BladeAtlasTexture>,
}

impl ops::Index<AtlasTextureKind> for BladeAtlasStorage {
    type Output = AtlasTextureList<BladeAtlasTexture>;
    fn index(&self, kind: AtlasTextureKind) -> &Self::Output {
        match kind {
            crate::AtlasTextureKind::Monochrome => &self.monochrome_textures,
            crate::AtlasTextureKind::Polychrome => &self.polychrome_textures,
        }
    }
}

impl ops::IndexMut<AtlasTextureKind> for BladeAtlasStorage {
    fn index_mut(&mut self, kind: AtlasTextureKind) -> &mut Self::Output {
        match kind {
            crate::AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
            crate::AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
        }
    }
}

impl BladeAtlasStorage {
    /// Borrows the texture behind `id`, or `None` when that slot is empty
    /// because the texture was destroyed (its index may since be reused).
    fn get(&self, id: AtlasTextureId) -> Option<&BladeAtlasTexture> {
        self[id.kind].textures.get(id.index as usize)?.as_ref()
    }

    fn destroy(&mut self, gpu: &gpu::Context) {
        for mut texture in self.monochrome_textures.drain().flatten() {
            texture.destroy(gpu);
        }
        for mut texture in self.polychrome_textures.drain().flatten() {
            texture.destroy(gpu);
        }
    }
}

struct BladeAtlasTexture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    raw: gpu::Texture,
    raw_view: gpu::TextureView,
    format: gpu::TextureFormat,
    live_atlas_keys: u32,
}

impl BladeAtlasTexture {
    fn allocate(&mut self, size: Size<DevicePixels>) -> Option<AtlasTile> {
        let allocation = self.allocator.allocate(size.into())?;
        let tile = AtlasTile {
            texture_id: self.id,
            tile_id: allocation.id.into(),
            padding: 0,
            bounds: Bounds {
                origin: allocation.rectangle.min.into(),
                size,
            },
        };
        self.live_atlas_keys += 1;
        Some(tile)
    }

    fn destroy(&mut self, gpu: &gpu::Context) {
        gpu.destroy_texture(self.raw);
        gpu.destroy_texture_view(self.raw_view);
    }

    fn bytes_per_pixel(&self) -> u8 {
        self.format.block_info().size
    }

    fn decrement_ref_count(&mut self) {
        self.live_atlas_keys -= 1;
    }

    fn is_unreferenced(&mut self) -> bool {
        self.live_atlas_keys == 0
    }
}

impl From<Size<DevicePixels>> for etagere::Size {
    fn from(size: Size<DevicePixels>) -> Self {
        etagere::Size::new(size.width.into(), size.height.into())
    }
}

impl From<etagere::Point> for Point<DevicePixels> {
    fn from(value: etagere::Point) -> Self {
        Point {
            x: DevicePixels::from(value.x),
            y: DevicePixels::from(value.y),
        }
    }
}

impl From<etagere::Size> for Size<DevicePixels> {
    fn from(size: etagere::Size) -> Self {
        Size {
            width: DevicePixels::from(size.width),
            height: DevicePixels::from(size.height),
        }
    }
}

impl From<etagere::Rectangle> for Bounds<DevicePixels> {
    fn from(rectangle: etagere::Rectangle) -> Self {
        Bounds {
            origin: rectangle.min.into(),
            size: rectangle.size().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AtlasTextureId, AtlasTextureKind, BladeAtlasStorage, retain_pending};

    fn id(index: u32, kind: AtlasTextureKind) -> AtlasTextureId {
        AtlasTextureId { index, kind }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Queued {
        id: AtlasTextureId,
        payload: u32,
    }

    // Regression: releasing an image between frames destroyed its atlas texture
    // while `initializations`/`uploads` still held its id, so the next
    // `flush_initializations` indexed a slot that had been emptied and panicked.
    #[test]
    fn discarding_a_destroyed_texture_drops_only_its_own_pending_work() {
        let dead = id(3, AtlasTextureKind::Polychrome);
        let mut pending = vec![
            Queued {
                id: id(1, AtlasTextureKind::Polychrome),
                payload: 10,
            },
            Queued {
                id: dead,
                payload: 11,
            },
            Queued {
                id: id(3, AtlasTextureKind::Monochrome),
                payload: 12,
            },
            Queued {
                id: id(2, AtlasTextureKind::Polychrome),
                payload: 13,
            },
        ];

        retain_pending(&mut pending, |entry| entry.id, dead);

        assert_eq!(
            pending,
            vec![
                Queued {
                    id: id(1, AtlasTextureKind::Polychrome),
                    payload: 10,
                },
                Queued {
                    id: id(3, AtlasTextureKind::Monochrome),
                    payload: 12,
                },
                Queued {
                    id: id(2, AtlasTextureKind::Polychrome),
                    payload: 13,
                },
            ],
            "entries for live textures must survive in their original order"
        );
    }

    #[test]
    fn discarding_an_untracked_texture_keeps_every_pending_entry() {
        let mut pending = vec![Queued {
            id: id(0, AtlasTextureKind::Monochrome),
            payload: 1,
        }];
        let before = vec![Queued {
            id: id(0, AtlasTextureKind::Monochrome),
            payload: 1,
        }];

        retain_pending(
            &mut pending,
            |entry| entry.id,
            id(7, AtlasTextureKind::Polychrome),
        );

        assert_eq!(pending, before);
    }

    #[test]
    fn emptied_texture_slots_are_reported_as_absent_instead_of_panicking() {
        let mut storage = BladeAtlasStorage::default();
        // `remove` destroys an unreferenced texture and leaves `None` behind so
        // the index can be reused by a later `push_texture`.
        storage.polychrome_textures.textures.push(None);
        storage.polychrome_textures.textures.push(None);

        assert!(storage.get(id(0, AtlasTextureKind::Polychrome)).is_none());
        assert!(storage.get(id(1, AtlasTextureKind::Polychrome)).is_none());
        assert!(storage.get(id(9, AtlasTextureKind::Polychrome)).is_none());
        assert!(storage.get(id(0, AtlasTextureKind::Monochrome)).is_none());
    }
}
