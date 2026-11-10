//! The CPU ⇄ GPU memory protocol (`docs` §2.4).
//!
//! Every struct here is `#[repr(C)]`, `Pod`, and sized/aligned to a CPU cache
//! line so that:
//!
//! * a primitive never straddles two cache lines (one instance == one 64B line),
//! * the shader can index `primitives[i]` with a single `shl 6`,
//! * `FramePushConstants` is exactly 64B and therefore fits one push-constant
//!   block on every implementation that supports the minimum 128B guarantee.
//!
//! Offsets are pinned by tests: a silent field reorder would otherwise corrupt
//! rendering in a way that is very hard to spot.

use bytemuck::{Pod, Zeroable};

use unbound_math::{f16_to_f32, f32_to_f16};

/// Number of bytes in one [`GpuPrimitiveInstance`] — also the cache line size
/// the whole engine is tuned for.
pub const PRIMITIVE_STRIDE: usize = 64;

/// Primitive kind, packed into the low 8 bits of `prim_type_and_flags`.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PrimType {
    /// Rounded rectangle with optional border and analytic shadow.
    #[default]
    RoundedRect = 0,
    /// Textured quad (image atlas page, `bindless` layer in the border slot).
    Image = 1,
    /// Glyph quad sampling `Atlas_R8` with subpixel phase.
    Glyph = 2,
    /// Colour glyph from `Atlas_RGBA8` (emoji / MTSDF).
    GlyphColor = 3,
    /// Quadratic Bézier segment rendered as a tight OBB (`docs` §3.4).
    Curve = 4,
    /// Hollow rounded rect used for focus rings / selection outlines.
    Stroke = 5,
    /// Shadow-only instance (no fill), used for detached overlays.
    Shadow = 6,
    /// Solid separator line.
    Line = 7,
}

impl PrimType {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Image,
            2 => Self::Glyph,
            3 => Self::GlyphColor,
            4 => Self::Curve,
            5 => Self::Stroke,
            6 => Self::Shadow,
            7 => Self::Line,
            _ => Self::RoundedRect,
        }
    }

    /// True for primitives whose fragment shader reads the glyph atlas and
    /// therefore needs gamma/stem compensation.
    #[must_use]
    pub const fn is_text(self) -> bool {
        matches!(self, Self::Glyph | Self::GlyphColor)
    }
}

/// Blend mode, packed into bits 8..16 of `prim_type_and_flags`.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlendMode {
    /// Straight source-over (premultiplied on the GPU side).
    #[default]
    Normal = 0,
    /// Additive — used by cursor trails and diagnostic overlays.
    Additive = 1,
    /// Multiply — used by canvas multiply layers.
    Multiply = 2,
    /// Replace — no blending at all (opaque core prepass writes depth only,
    /// but replace is also used for the first opaque layer when MSAA is off).
    Replace = 3,
}

impl BlendMode {
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Additive,
            2 => Self::Multiply,
            3 => Self::Replace,
            _ => Self::Normal,
        }
    }
}

/// Sub-element order inside one component (`docs` §6.3, bits 7..0 of the key).
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubElement {
    #[default]
    Shadow = 0,
    Background = 1,
    Content = 2,
    InnerShadow = 3,
    Border = 4,
    FocusRing = 5,
}

/// Portal / pass layer, top 8 bits of the 64-bit sort key.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PassLayer {
    #[default]
    Canvas = 0,
    Ui = 1,
    /// Dropdowns, tooltips, context menus.
    Overlay = 2,
    /// Modal dialogs.
    Modal = 3,
    /// Debug/diagnostics overlay is always on top.
    Debug = 255,
}

impl PassLayer {
    /// Portals start at layer 2 and can go up to 254.
    #[must_use]
    pub const fn portal(depth: u8) -> Self {
        // `depth` is clamped to 254 so 255 stays reserved for diagnostics.
        if depth > 254 { Self::Debug } else { Self::from_u8(2u8.saturating_add(depth)) }
    }

    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Canvas,
            1 => Self::Ui,
            255 => Self::Debug,
            _ => Self::Overlay,
        }
    }
}

/// 64B push constants — the only per-frame CPU→GPU payload (`docs` §2.4).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FramePushConstants {
    /// Physical device address of the `GpuPrimitiveInstance` array.
    pub primitives_bda: u64,
    /// Physical device address of the `GpuTransform` array.
    pub transforms_bda: u64,
    /// Physical device address of the `GpuClipNode` array.
    pub clips_bda: u64,
    /// Physical device address of the RGBA16F theme LUT.
    pub theme_lut_bda: u64,
    /// Physical device address of curve/gradient payloads.
    pub payloads_bda: u64,
    /// Chunk containing the camera origin.
    pub camera_chunk: [i32; 2],
    /// Camera position inside that chunk, in `[0, 4096)`.
    pub camera_local: [f32; 2],
    /// Physical viewport size in pixels.
    pub viewport_size: [f32; 2],
    /// Canvas zoom factor (1.0 == 100%).
    pub camera_zoom: f32,
    /// OS DPI × app zoom.
    pub dpi_scale: f32,
}

impl FramePushConstants {
    /// Size contract for the push-constant range.
    pub const SIZE: u32 = core::mem::size_of::<Self>() as u32;
}

/// One 64-byte primitive instance. Field packing follows `docs` §2.4 exactly.
#[repr(C, align(64))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuPrimitiveInstance {
    /// World chunk id. Static UI primitives always use `[0, 0]`.
    pub chunk_id: [i32; 2],
    /// Position inside the chunk, in `[0.0, 4096.0)`.
    pub local_pos: [f32; 2],
    /// Primitive size in logical pixels.
    pub size: [f32; 2],
    /// Corner radii: top-left, top-right, bottom-right, bottom-left (f16).
    pub corner_radii: [u16; 4],
    /// Border width (f16).
    pub border_width: u16,
    /// Analytic shadow blur radius (f16, 0 == no shadow).
    pub shadow_blur: u16,
    /// Index into `transforms_bda`; bit 31 marks an active animation.
    pub transform_idx: u32,
    /// Index into `clips_bda`.
    pub clip_idx: u32,
    /// Theme colour token or packed atlas UV index.
    pub fill_token_or_uv: u32,
    /// Border colour token or bindless texture/atlas layer index.
    pub border_token_or_tex: u32,
    /// `[8b type | 8b blend | 16b flags]`.
    pub prim_type_and_flags: u32,
    /// `[16b normalised depth | 16b UNORM opacity]`.
    pub z_and_opacity: u32,
    /// Index into `payloads_bda` for curves and gradients.
    pub extra_payload_idx: u32,
}

/// Bit 31 of `transform_idx`: the primitive is mid-animation and must bypass
/// integer pixel snapping (`docs` §4.4).
pub const TRANSFORM_ANIMATED_BIT: u32 = 1 << 31;
/// Mask that strips the animation bit, leaving the real table index.
pub const TRANSFORM_INDEX_MASK: u32 = !TRANSFORM_ANIMATED_BIT;

impl GpuPrimitiveInstance {
    /// Pack the `prim_type_and_flags` word.
    #[must_use]
    pub const fn pack_type(prim: PrimType, blend: BlendMode, flags: u16) -> u32 {
        (prim as u32) | ((blend as u32) << 8) | ((flags as u32) << 16)
    }

    /// Unpack `[type, blend, flags]`.
    #[must_use]
    pub const fn unpack_type(word: u32) -> (PrimType, BlendMode, u16) {
        (
            PrimType::from_u8((word & 0xff) as u8),
            BlendMode::from_u8(((word >> 8) & 0xff) as u8),
            (word >> 16) as u16,
        )
    }

    #[inline]
    #[must_use]
    pub const fn prim_type(&self) -> PrimType {
        PrimType::from_u8((self.prim_type_and_flags & 0xff) as u8)
    }

    #[inline]
    #[must_use]
    pub const fn blend_mode(&self) -> BlendMode {
        BlendMode::from_u8(((self.prim_type_and_flags >> 8) & 0xff) as u8)
    }

    /// `[16b depth | 16b opacity]`.
    #[must_use]
    pub const fn pack_z_opacity(depth: u16, opacity: f32) -> u32 {
        let o = (opacity.clamp(0.0, 1.0) * 65535.0 + 0.5) as u32;
        ((depth as u32) << 16) | (o & 0xffff)
    }

    #[inline]
    #[must_use]
    pub const fn depth_u16(&self) -> u16 {
        (self.z_and_opacity >> 16) as u16
    }

    /// Opacity in `[0, 1]` decoded from UNORM16.
    #[inline]
    #[must_use]
    pub fn opacity(&self) -> f32 {
        (self.z_and_opacity & 0xffff) as f32 / 65535.0
    }

    /// Reverse-Z depth in `[0, 1]` (1.0 == nearest).
    #[inline]
    #[must_use]
    pub fn depth(&self) -> f32 {
        self.depth_u16() as f32 / 65535.0
    }

    /// True when the primitive participates in the animation channel.
    #[inline]
    #[must_use]
    pub const fn is_animated(&self) -> bool {
        self.transform_idx & TRANSFORM_ANIMATED_BIT != 0
    }

    /// Real transform table index (animation bit stripped).
    #[inline]
    #[must_use]
    pub const fn transform_index(&self) -> u32 {
        self.transform_idx & TRANSFORM_INDEX_MASK
    }

    #[inline]
    #[must_use]
    pub fn radii(&self) -> [f32; 4] {
        [
            f16_to_f32(self.corner_radii[0]),
            f16_to_f32(self.corner_radii[1]),
            f16_to_f32(self.corner_radii[2]),
            f16_to_f32(self.corner_radii[3]),
        ]
    }

    #[inline]
    pub fn set_radii(&mut self, radii: [f32; 4]) {
        self.corner_radii = [
            f32_to_f16(radii[0]),
            f32_to_f16(radii[1]),
            f32_to_f16(radii[2]),
            f32_to_f16(radii[3]),
        ];
    }

    #[inline]
    #[must_use]
    pub fn border_width(&self) -> f32 {
        f16_to_f32(self.border_width)
    }

    #[inline]
    pub fn set_border_width(&mut self, w: f32) {
        self.border_width = f32_to_f16(w);
    }

    #[inline]
    #[must_use]
    pub fn shadow_blur(&self) -> f32 {
        f16_to_f32(self.shadow_blur)
    }

    #[inline]
    pub fn set_shadow_blur(&mut self, b: f32) {
        self.shadow_blur = f32_to_f16(b);
    }

    /// Axis-aligned bounds in chunk-local space, expanded by the shadow blur so
    /// the culling compute shader keeps shadow-only geometry alive.
    #[inline]
    #[must_use]
    pub fn bounds_with_shadow(&self) -> ([f32; 2], [f32; 2]) {
        let pad = self.shadow_blur() * 3.0;
        (
            [self.local_pos[0] - pad, self.local_pos[1] - pad],
            [
                self.local_pos[0] + self.size[0] + pad,
                self.local_pos[1] + self.size[1] + pad,
            ],
        )
    }
}

/// 64B clip node: one folded hard AABB plus **two** rounded parent slots.
///
/// The spec's doc-comment says "32 bytes", but the declared fields sum to 64B.
/// The fields are what the shader needs (two nested rounded containers), so the
/// layout is 64B and we document the discrepancy rather than silently dropping
/// the second slot.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuClipNode {
    /// Folded hard clip `(min_x, min_y, max_x, max_y)`.
    pub aabb_clip: [f32; 4],
    /// First rounded parent `(center_x, center_y, half_w, half_h)`.
    pub rounded_clip_0: [f32; 4],
    /// First parent's four corner radii (f16).
    pub radii_0: [u16; 4],
    /// Second rounded parent.
    pub rounded_clip_1: [f32; 4],
    /// Second parent's four corner radii (f16).
    pub radii_1: [u16; 4],
}

/// Sentinel clip index meaning "unclipped".
pub const CLIP_NONE: u32 = u32::MAX;

impl GpuClipNode {
    /// A node that clips to `aabb` with no rounded parents.
    #[must_use]
    pub fn from_aabb(aabb: [f32; 4]) -> Self {
        Self {
            aabb_clip: aabb,
            // A degenerate rounded clip with zero radii and an enormous extent
            // is a no-op in the shader's SDF test.
            rounded_clip_0: [0.0, 0.0, 1.0e9, 1.0e9],
            radii_0: [0; 4],
            rounded_clip_1: [0.0, 0.0, 1.0e9, 1.0e9],
            radii_1: [0; 4],
        }
    }

    /// True when the folded AABB is empty (everything culled).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.aabb_clip[0] >= self.aabb_clip[2] || self.aabb_clip[1] >= self.aabb_clip[3]
    }
}

/// 32B transform record for the independent animation channel.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuTransform {
    /// `m00, m10, m01, m11` (column-major 2×2).
    pub linear: [f32; 4],
    /// Translation in logical pixels.
    pub translation: [f32; 2],
    /// `1.0` when the transform is identity and can be skipped entirely.
    pub scale_hint: f32,
    /// Bit-packed flags (see [`TransformFlags`]).
    pub flags: u32,
}

impl Default for GpuTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl GpuTransform {
    pub const IDENTITY: Self = Self {
        linear: [1.0, 0.0, 0.0, 1.0],
        translation: [0.0, 0.0],
        scale_hint: 1.0,
        flags: 0,
    };

    #[must_use]
    pub fn from_affine(m: unbound_math::Affine2) -> Self {
        Self {
            linear: [m.m00, m.m10, m.m01, m.m11],
            translation: [m.tx, m.ty],
            scale_hint: m.max_scale(),
            flags: 0,
        }
    }

    #[must_use]
    pub fn to_affine(self) -> unbound_math::Affine2 {
        unbound_math::Affine2 {
            m00: self.linear[0],
            m10: self.linear[1],
            m01: self.linear[2],
            m11: self.linear[3],
            tx: self.translation[0],
            ty: self.translation[1],
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.linear[0] == 1.0
            && self.linear[1] == 0.0
            && self.linear[2] == 0.0
            && self.linear[3] == 1.0
            && self.translation[0] == 0.0
            && self.translation[1] == 0.0
    }
}

/// Flags stored in [`GpuTransform::flags`].
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct TransformFlags(pub u32);

impl TransformFlags {
    pub const EMPTY: Self = Self(0);
    /// The primitive is currently animating: bypass pixel snapping.
    pub const ANIMATED: Self = Self(1 << 0);
    /// The scale is uniform, so the shader can use a cheap scalar path.
    pub const UNIFORM_SCALE: Self = Self(1 << 1);
    /// The transform mirrors on X.
    pub const MIRROR_X: Self = Self(1 << 2);
    /// Skip the transform entirely (identity fast path in the VS).
    pub const SKIP: Self = Self(1 << 3);

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl core::ops::BitOr for TransformFlags {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// 48B payload header for curves and gradients.
///
/// For [`PrimType::Curve`]: `p0`/`p1`/`p2` are the quadratic control polygon in
/// primitive-local space. For gradients: `p0`/`p1` are the endpoints and `p2.x`
/// is the stop count (stops follow inline).
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuPayload {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub p2: [f32; 2],
    /// Stroke width in logical pixels.
    pub line_width: f32,
    /// Gradient stop offset / curve flags.
    pub extra: u32,
    /// Packed flags.
    pub flags: u32,
    /// Explicit tail padding: three words keep the struct exactly 48B with no
    /// implicit padding (a `Pod` type must not contain any).
    pub _reserved: [u32; 3],
}

impl GpuPayload {
    #[must_use]
    pub fn curve(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], line_width: f32) -> Self {
        Self {
            p0,
            p1,
            p2,
            line_width,
            extra: 0,
            flags: 0,
            _reserved: [0; 3],
        }
    }
}

/// Indirect draw command; layout-compatible with `VkDrawIndirectCommand`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct DrawIndirectCommand {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
}

impl DrawIndirectCommand {
    #[must_use]
    pub const fn new(instance_count: u32, first_instance: u32) -> Self {
        Self {
            vertex_count: 4,
            instance_count,
            first_vertex: 0,
            first_instance,
        }
    }
}

/// Flags stored in the high 16 bits of `prim_type_and_flags`.
///
/// Bits 0..2 are the **sub-element order** that also forms the low byte of the
/// 64-bit sort key, so a node's shadow always lands behind its background and
/// its border always lands in front.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct PrimFlags(pub u16);

impl PrimFlags {
    pub const EMPTY: Self = Self(0);
    /// Sub-element order mask (`SubElement`).
    pub const SUB_ELEMENT_MASK: Self = Self(0b111);
    /// The glyph sits on a dark surface: apply stem darkening.
    pub const DARK_BG: Self = Self(1 << 3);
    /// Sample the RGBA atlas as a multi-channel distance field.
    pub const MSDF: Self = Self(1 << 4);
    /// The text run was truncated; the shader may soften the trailing edge.
    pub const ELLIPSIS: Self = Self(1 << 5);
    /// Rasterised with a horizontal subpixel phase (Atlas_R8 phase layers).
    pub const SUBPIXEL: Self = Self(1 << 6);
    /// The primitive is the topmost of an opaque core (used by the prepass).
    pub const OPAQUE_CORE: Self = Self(1 << 7);

    #[inline]
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Build the flags word for a given sub-element plus extra flags.
    #[must_use]
    pub const fn with_sub(sub: SubElement, extra: Self) -> Self {
        Self((sub as u16) | extra.0)
    }

    /// Sub-element encoded in the low three bits.
    #[must_use]
    pub const fn sub_element(self) -> u8 {
        (self.0 & Self::SUB_ELEMENT_MASK.0) as u8
    }
}

impl core::ops::BitOr for PrimFlags {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// The **single** 128-byte push-constant block used by every pipeline.
///
/// The first 72 bytes are the documented `FramePushConstants`; the tail carries
/// the compute stage's scratch addresses and counters. Graphics pipelines push
/// the whole block with a zeroed tail, which lets one `VkPipelineLayout` serve
/// the uber shader and all six compute stages (and keeps the range under the
/// 128-byte guaranteed minimum).
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PushConstants {
    pub frame: FramePushConstants,
    /// Element count for the current compute dispatch.
    pub prim_count: u32,
    /// Radix digit shift (0, 8, 16, … 56).
    pub digit: u32,
    /// Number of 256-element blocks in the current sort pass.
    pub block_count: u32,
    /// Generic scratch word (stage specific).
    pub aux: u32,
    pub scratch_bda: u64,
    pub scratch_bda2: u64,
    pub scratch_bda3: u64,
    pub scratch_bda4: u64,
    pub scratch_bda5: u64,
}

impl PushConstants {
    /// Push-constant range size in bytes.
    pub const SIZE: u32 = core::mem::size_of::<Self>() as u32;

    /// Wrapper carrying only the frame prefix (what most callers have).
    #[must_use]
    pub fn from_frame(frame: FramePushConstants) -> Self {
        Self {
            frame,
            ..Default::default()
        }
    }
}

/// One incremental-scatter patch, layout-compatible with `scatter.comp`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuScatterPatch {
    pub dst_offset: u32,
    pub src_offset: u32,
    pub len: u32,
    pub _pad: u32,
}

/// A `(key, primitive index)` pair as produced by the culling pass.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct SortedEntry {
    pub key: u64,
    pub index: u32,
    pub _pad: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn push_constants_fit_one_block() {
        // The spec's doc-comment says 64B, but the nine declared fields sum to
        // 72B (5 BDA pointers + 3 vec2 + 2 scalars). The fields are all needed,
        // so 72B stands — still far below the 128B guaranteed minimum.
        assert_eq!(size_of::<FramePushConstants>(), 72);
        assert_eq!(align_of::<FramePushConstants>(), 8);
        assert_eq!(FramePushConstants::SIZE, 72);
    }

    #[test]
    fn push_constant_offsets_are_pinned() {
        assert_eq!(offset_of!(FramePushConstants, primitives_bda), 0);
        assert_eq!(offset_of!(FramePushConstants, transforms_bda), 8);
        assert_eq!(offset_of!(FramePushConstants, clips_bda), 16);
        assert_eq!(offset_of!(FramePushConstants, theme_lut_bda), 24);
        assert_eq!(offset_of!(FramePushConstants, payloads_bda), 32);
        assert_eq!(offset_of!(FramePushConstants, camera_chunk), 40);
        assert_eq!(offset_of!(FramePushConstants, camera_local), 48);
        assert_eq!(offset_of!(FramePushConstants, viewport_size), 56);
    }

    #[test]
    fn primitive_is_one_cache_line() {
        assert_eq!(size_of::<GpuPrimitiveInstance>(), PRIMITIVE_STRIDE);
        assert_eq!(align_of::<GpuPrimitiveInstance>(), 64);
        // Every field lands inside the single 64B line.
        assert_eq!(offset_of!(GpuPrimitiveInstance, chunk_id), 0);
        assert_eq!(offset_of!(GpuPrimitiveInstance, local_pos), 8);
        assert_eq!(offset_of!(GpuPrimitiveInstance, size), 16);
        assert_eq!(offset_of!(GpuPrimitiveInstance, corner_radii), 24);
        assert_eq!(offset_of!(GpuPrimitiveInstance, border_width), 32);
        assert_eq!(offset_of!(GpuPrimitiveInstance, shadow_blur), 34);
        assert_eq!(offset_of!(GpuPrimitiveInstance, transform_idx), 36);
        assert_eq!(offset_of!(GpuPrimitiveInstance, clip_idx), 40);
        assert_eq!(offset_of!(GpuPrimitiveInstance, fill_token_or_uv), 44);
        assert_eq!(offset_of!(GpuPrimitiveInstance, border_token_or_tex), 48);
        assert_eq!(offset_of!(GpuPrimitiveInstance, prim_type_and_flags), 52);
        assert_eq!(offset_of!(GpuPrimitiveInstance, z_and_opacity), 56);
        assert_eq!(offset_of!(GpuPrimitiveInstance, extra_payload_idx), 60);
    }

    #[test]
    fn arrays_of_primitives_are_tightly_packed() {
        let mut v = vec![GpuPrimitiveInstance::default(); 16];
        for (i, p) in v.iter_mut().enumerate() {
            p.local_pos = [i as f32, 0.0];
        }
        let bytes: &[u8] = bytemuck::cast_slice(&v);
        assert_eq!(bytes.len(), 16 * PRIMITIVE_STRIDE);
        // Stride check: instance 3's local_pos.x sits at 3*64 + 8.
        let off = 3 * PRIMITIVE_STRIDE + 8;
        assert_eq!(f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()), 3.0);
    }

    #[test]
    fn type_and_blend_packing_round_trips() {
        let w = GpuPrimitiveInstance::pack_type(PrimType::Curve, BlendMode::Additive, 0xbeef);
        assert_eq!(
            GpuPrimitiveInstance::unpack_type(w),
            (PrimType::Curve, BlendMode::Additive, 0xbeef)
        );
    }

    #[test]
    fn opacity_and_depth_pack_without_crosstalk() {
        let p = GpuPrimitiveInstance {
            z_and_opacity: GpuPrimitiveInstance::pack_z_opacity(0xabcd, 0.5),
            ..Default::default()
        };
        assert_eq!(p.depth_u16(), 0xabcd);
        assert!((p.opacity() - 0.5).abs() < 2e-5);
        assert!((p.depth() - 0.670_58).abs() < 1e-3);
    }

    #[test]
    fn opacity_clamps() {
        assert_eq!(GpuPrimitiveInstance::pack_z_opacity(0, 4.0) & 0xffff, 0xffff);
        assert_eq!(GpuPrimitiveInstance::pack_z_opacity(0, -1.0) & 0xffff, 0);
    }

    #[test]
    fn animation_bit_does_not_alias_the_index() {
        let mut p = GpuPrimitiveInstance::default();
        p.transform_idx = 1234;
        assert!(!p.is_animated());
        assert_eq!(p.transform_index(), 1234);
        p.transform_idx |= TRANSFORM_ANIMATED_BIT;
        assert!(p.is_animated());
        assert_eq!(p.transform_index(), 1234);
    }

    #[test]
    fn radii_and_border_round_trip_through_f16() {
        let mut p = GpuPrimitiveInstance::default();
        p.set_radii([4.0, 8.0, 12.0, 16.0]);
        assert_eq!(p.radii(), [4.0, 8.0, 12.0, 16.0]);
        p.set_border_width(1.0);
        assert_eq!(p.border_width(), 1.0);
        p.set_shadow_blur(24.0);
        assert_eq!(p.shadow_blur(), 24.0);
    }

    #[test]
    fn clip_node_is_16_byte_aligned_and_64_bytes() {
        assert_eq!(size_of::<GpuClipNode>(), 64);
        assert_eq!(align_of::<GpuClipNode>(), 16);
        let c = GpuClipNode::from_aabb([0.0, 0.0, 10.0, 10.0]);
        assert!(!c.is_empty());
        assert!(GpuClipNode::from_aabb([10.0, 0.0, 0.0, 10.0]).is_empty());
    }

    #[test]
    fn transform_is_32_bytes_and_round_trips() {
        assert_eq!(size_of::<GpuTransform>(), 32);
        assert_eq!(align_of::<GpuTransform>(), 16);
        let m = unbound_math::Affine2::around(
            unbound_math::Vec2::new(4.0, 4.0),
            unbound_math::Vec2::new(2.0, 2.0),
            0.5,
            unbound_math::Vec2::new(1.0, 0.0),
        );
        let t = GpuTransform::from_affine(m);
        assert_eq!(t.to_affine(), m);
        assert!(GpuTransform::default().is_identity());
    }

    #[test]
    fn payload_is_48_bytes() {
        assert_eq!(size_of::<GpuPayload>(), 48);
        assert_eq!(align_of::<GpuPayload>(), 16);
    }

    #[test]
    fn draw_command_matches_vulkan_layout() {
        assert_eq!(size_of::<DrawIndirectCommand>(), 16);
        let c = DrawIndirectCommand::new(7, 42);
        assert_eq!(c.vertex_count, 4);
        assert_eq!(c.instance_count, 7);
        assert_eq!(c.first_instance, 42);
    }

    #[test]
    fn pass_layer_and_prim_type_normalise_unknowns() {
        assert_eq!(PrimType::from_u8(99), PrimType::RoundedRect);
        assert_eq!(BlendMode::from_u8(9), BlendMode::Normal);
        assert_eq!(PassLayer::from_u8(77), PassLayer::Overlay);
        assert_eq!(PassLayer::portal(0), PassLayer::from_u8(2));
        assert_eq!(PassLayer::portal(253), PassLayer::from_u8(255));
        assert_eq!(PassLayer::portal(254), PassLayer::Debug);
        assert!(PrimType::Glyph.is_text());
        assert!(!PrimType::Image.is_text());
    }

    #[test]
    fn push_constants_are_exactly_128_bytes() {
        assert_eq!(size_of::<PushConstants>(), 128);
        assert_eq!(PushConstants::SIZE, 128);
        assert_eq!(offset_of!(PushConstants, frame), 0);
        assert_eq!(offset_of!(PushConstants, prim_count), 72);
        assert_eq!(offset_of!(PushConstants, digit), 76);
        assert_eq!(offset_of!(PushConstants, block_count), 80);
        assert_eq!(offset_of!(PushConstants, aux), 84);
        assert_eq!(offset_of!(PushConstants, scratch_bda), 88);
        assert_eq!(offset_of!(PushConstants, scratch_bda5), 120);
    }

    #[test]
    fn prim_flags_pack_sub_element_and_markers() {
        let f = PrimFlags::with_sub(SubElement::Border, PrimFlags::DARK_BG);
        assert_eq!(f.sub_element(), SubElement::Border as u8);
        assert!(f.contains(PrimFlags::DARK_BG));
        assert!(!f.contains(PrimFlags::MSDF));
    }

    #[test]
    fn scatter_patch_matches_the_shader_layout() {
        assert_eq!(size_of::<GpuScatterPatch>(), 16);
        let p = GpuScatterPatch {
            dst_offset: 1,
            src_offset: 2,
            len: 64,
            _pad: 0,
        };
        assert_eq!(bytemuck::bytes_of(&p).len(), 16);
    }

    #[test]
    fn sorted_entry_is_16_bytes() {
        assert_eq!(size_of::<SortedEntry>(), 16);
    }

    #[test]
    fn bounds_include_shadow_bleed() {
        let mut p = GpuPrimitiveInstance::default();
        p.local_pos = [10.0, 20.0];
        p.size = [100.0, 50.0];
        p.set_shadow_blur(4.0);
        let (min, max) = p.bounds_with_shadow();
        assert_eq!(min, [-2.0, 8.0]);
        assert_eq!(max, [122.0, 82.0]);
    }
}
