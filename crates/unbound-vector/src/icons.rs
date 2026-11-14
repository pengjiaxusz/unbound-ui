//! 内置 Material 风格矢量图标与 `cha-set` 22 类文件类型图标注册表（阶段 1 WP3）。
//!
//! 所有图标均以标准 SVG 路径指令定义（`24×24` 规范视口），可直接交由 [`crate::parse_svg_path`]
//! 解析并降阶为 GPU 二次贝塞尔曲线段，无需外部位图依赖，在任意 DPI 与画布缩放下保持绝对锐利。

use crate::svg_path::{parse_svg_path, SvgPath};

/// 内置 Material 风格通用 UI 与调试器矢量图标。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MaterialIcon {
    Search,
    Close,
    Check,
    ChevronRight,
    ChevronDown,
    ChevronLeft,
    ChevronUp,
    Folder,
    FolderOpen,
    File,
    Copy,
    Trash,
    Refresh,
    Settings,
    Filter,
    Play,
    Pause,
    Camera,
    Bug,
    Layers,
    Cube,
    Image,
    Code,
    Terminal,
    Warning,
    Info,
    Error,
    Pin,
    Eye,
    EyeOff,
    Plus,
    Minus,
}

impl MaterialIcon {
    /// 全部内置 Material 图标。
    pub const ALL: [Self; 32] = [
        Self::Search,
        Self::Close,
        Self::Check,
        Self::ChevronRight,
        Self::ChevronDown,
        Self::ChevronLeft,
        Self::ChevronUp,
        Self::Folder,
        Self::FolderOpen,
        Self::File,
        Self::Copy,
        Self::Trash,
        Self::Refresh,
        Self::Settings,
        Self::Filter,
        Self::Play,
        Self::Pause,
        Self::Camera,
        Self::Bug,
        Self::Layers,
        Self::Cube,
        Self::Image,
        Self::Code,
        Self::Terminal,
        Self::Warning,
        Self::Info,
        Self::Error,
        Self::Pin,
        Self::Eye,
        Self::EyeOff,
        Self::Plus,
        Self::Minus,
    ];

    /// 返回 `24×24` 规范坐标系下的 SVG 路径字符串。
    #[must_use]
    pub const fn svg_path(self) -> &'static str {
        match self {
            Self::Search => "M 11 4 C 14.866 4 18 7.134 18 11 C 18 14.866 14.866 18 11 18 C 7.134 18 4 14.866 4 11 C 4 7.134 7.134 4 11 4 Z M 16.5 16.5 L 20.5 20.5",
            Self::Close => "M 6 6 L 18 18 M 18 6 L 6 18",
            Self::Check => "M 5 12.5 L 10 17.5 L 19 6.5",
            Self::ChevronRight => "M 9 6 L 15 12 L 9 18",
            Self::ChevronDown => "M 6 9 L 12 15 L 18 9",
            Self::ChevronLeft => "M 15 6 L 9 12 L 15 18",
            Self::ChevronUp => "M 6 15 L 12 9 L 18 15",
            Self::Folder => "M 3 6 L 9 6 L 11 8 L 21 8 L 21 19 L 3 19 Z",
            Self::FolderOpen => "M 3 6 L 9 6 L 11 8 L 21 8 L 21 11 L 5 11 L 3 19 L 19 19 L 21 11",
            Self::File => "M 6 3 L 14 3 L 19 8 L 19 21 L 6 21 Z M 14 3 L 14 8 L 19 8",
            Self::Copy => "M 8 8 L 19 8 L 19 20 L 8 20 Z M 5 16 L 5 4 L 16 4",
            Self::Trash => "M 4 7 L 20 7 M 9 7 L 9 4 L 15 4 L 15 7 M 6 7 L 7 20 L 17 20 L 18 7",
            Self::Refresh => "M 19 8 C 17.5 5 14 4 10.5 5 C 6.5 6 4 10 5 14 C 6 18 10 20 14 19 C 17 18 19 15.5 19.5 13 M 19 4 L 19 8 L 15 8",
            Self::Settings => "M 12 8 C 14.2 8 16 9.8 16 12 C 16 14.2 14.2 16 12 16 C 9.8 16 8 14.2 8 12 C 8 9.8 9.8 8 12 8 Z M 12 3 L 12 5 M 12 19 L 12 21 M 3 12 L 5 12 M 19 12 L 21 12",
            Self::Filter => "M 4 5 L 20 5 L 14 12 L 14 19 L 10 17 L 10 12 Z",
            Self::Play => "M 7 5 L 19 12 L 7 19 Z",
            Self::Pause => "M 7 5 L 10 5 L 10 19 L 7 19 Z M 14 5 L 17 5 L 17 19 L 14 19 Z",
            Self::Camera => "M 4 8 L 8 8 L 10 5 L 14 5 L 16 8 L 20 8 L 20 19 L 4 19 Z M 12 10 C 13.65 10 15 11.35 15 13 C 15 14.65 13.65 16 12 16 C 10.35 16 9 14.65 9 13 C 9 11.35 10.35 10 12 10 Z",
            Self::Bug => "M 8 9 L 16 9 L 16 16 C 16 18.2 14.2 20 12 20 C 9.8 20 8 18.2 8 16 Z M 5 11 L 8 11 M 16 11 L 19 11 M 5 15 L 8 15 M 16 15 L 19 15 M 9 6 L 15 6",
            Self::Layers => "M 12 3 L 21 8 L 12 13 L 3 8 Z M 3 12 L 12 17 L 21 12 M 3 16 L 12 21 L 21 16",
            Self::Cube => "M 12 3 L 20 7.5 L 20 16.5 L 12 21 L 4 16.5 L 4 7.5 Z M 12 12 L 20 7.5 M 12 12 L 4 7.5 M 12 12 L 12 21",
            Self::Image => "M 4 5 L 20 5 L 20 19 L 4 19 Z M 8.5 10 C 9.3 10 10 9.3 10 8.5 C 10 7.7 9.3 7 8.5 7 C 7.7 7 7 7.7 7 8.5 C 7 9.3 7.7 10 8.5 10 Z M 4 17 L 10 12 L 14 15 L 17 13 L 20 16",
            Self::Code => "M 8 7 L 3 12 L 8 17 M 16 7 L 21 12 L 16 17 M 14 4 L 10 20",
            Self::Terminal => "M 4 6 L 10 12 L 4 18 M 12 18 L 20 18",
            Self::Warning => "M 12 4 L 21 19 L 3 19 Z M 12 10 L 12 14 M 12 16.5 L 12 17",
            Self::Info => "M 12 4 C 16.4 4 20 7.6 20 12 C 20 16.4 16.4 20 12 20 C 7.6 20 4 16.4 4 12 C 4 7.6 7.6 4 12 4 Z M 12 11 L 12 16 M 12 8 L 12 8.5",
            Self::Error => "M 12 4 C 16.4 4 20 7.6 20 12 C 20 16.4 16.4 20 12 20 C 7.6 20 4 16.4 4 12 C 4 7.6 7.6 4 12 4 Z M 9 9 L 15 15 M 15 9 L 9 15",
            Self::Pin => "M 9 4 L 15 4 L 14 11 L 17 14 L 7 14 L 10 11 Z M 12 14 L 12 20",
            Self::Eye => "M 2 12 C 5 7 9 5 12 5 C 15 5 19 7 22 12 C 19 17 15 19 12 19 C 9 19 5 17 2 12 Z M 12 9 C 13.65 9 15 10.35 15 12 C 15 13.65 13.65 15 12 15 C 10.35 15 9 13.65 9 12 C 9 10.35 10.35 9 12 9 Z",
            Self::EyeOff => "M 3 3 L 21 21 M 2 12 C 5 7 9 5 12 5 C 15 5 19 7 22 12 C 19 17 15 19 12 19 C 9 19 5 17 2 12 Z",
            Self::Plus => "M 12 5 L 12 19 M 5 12 L 19 12",
            Self::Minus => "M 5 12 L 19 12",
        }
    }

    /// 解析为 [`SvgPath`]。
    ///
    /// 内置图标路径按 `0.25px` 拟合容差降阶，与 `SvgPath::parse` 的调用方保持一致。
    #[must_use]
    pub fn parsed_path(self) -> SvgPath {
        parse_svg_path(self.svg_path(), 0.25).expect("built-in MaterialIcon SVG path must be valid")
    }
}

/// 与 `cha-set` 对齐的 22 类专业文件与图形资产图标。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileTypeIcon {
    Folder,
    GenericFile,
    Image,
    TextureKtx2,
    TextureDds,
    MeshObj,
    MeshFbx,
    MeshGltf,
    ShaderHlsl,
    ShaderGlsl,
    ShaderSpirv,
    Material,
    Scene,
    Audio,
    Video,
    Archive,
    CodeRust,
    CodeJson,
    CodeToml,
    CaptureRdc,
    Log,
    Executable,
}

impl FileTypeIcon {
    /// 全部 22 类文件类型图标。
    pub const ALL: [Self; 22] = [
        Self::Folder,
        Self::GenericFile,
        Self::Image,
        Self::TextureKtx2,
        Self::TextureDds,
        Self::MeshObj,
        Self::MeshFbx,
        Self::MeshGltf,
        Self::ShaderHlsl,
        Self::ShaderGlsl,
        Self::ShaderSpirv,
        Self::Material,
        Self::Scene,
        Self::Audio,
        Self::Video,
        Self::Archive,
        Self::CodeRust,
        Self::CodeJson,
        Self::CodeToml,
        Self::CaptureRdc,
        Self::Log,
        Self::Executable,
    ];

    /// 根据文件扩展名（忽略大小写与前导点号）推断对应的文件类型图标。
    #[must_use]
    pub fn from_extension(ext: &str) -> Self {
        let cleaned = ext.trim().trim_start_matches('.');
        if cleaned.eq_ignore_ascii_case("ktx2") || cleaned.eq_ignore_ascii_case("ktx") {
            Self::TextureKtx2
        } else if cleaned.eq_ignore_ascii_case("dds")
            || cleaned.eq_ignore_ascii_case("hdr")
            || cleaned.eq_ignore_ascii_case("exr")
        {
            Self::TextureDds
        } else if cleaned.eq_ignore_ascii_case("png")
            || cleaned.eq_ignore_ascii_case("jpg")
            || cleaned.eq_ignore_ascii_case("jpeg")
            || cleaned.eq_ignore_ascii_case("webp")
            || cleaned.eq_ignore_ascii_case("tga")
            || cleaned.eq_ignore_ascii_case("bmp")
        {
            Self::Image
        } else if cleaned.eq_ignore_ascii_case("obj") {
            Self::MeshObj
        } else if cleaned.eq_ignore_ascii_case("fbx") {
            Self::MeshFbx
        } else if cleaned.eq_ignore_ascii_case("gltf") || cleaned.eq_ignore_ascii_case("glb") {
            Self::MeshGltf
        } else if cleaned.eq_ignore_ascii_case("hlsl") || cleaned.eq_ignore_ascii_case("fx") {
            Self::ShaderHlsl
        } else if cleaned.eq_ignore_ascii_case("glsl")
            || cleaned.eq_ignore_ascii_case("vert")
            || cleaned.eq_ignore_ascii_case("frag")
            || cleaned.eq_ignore_ascii_case("comp")
            || cleaned.eq_ignore_ascii_case("wgsl")
        {
            Self::ShaderGlsl
        } else if cleaned.eq_ignore_ascii_case("spv") || cleaned.eq_ignore_ascii_case("dxil") {
            Self::ShaderSpirv
        } else if cleaned.eq_ignore_ascii_case("mat") || cleaned.eq_ignore_ascii_case("mtl") {
            Self::Material
        } else if cleaned.eq_ignore_ascii_case("scene")
            || cleaned.eq_ignore_ascii_case("usd")
            || cleaned.eq_ignore_ascii_case("usda")
        {
            Self::Scene
        } else if cleaned.eq_ignore_ascii_case("wav")
            || cleaned.eq_ignore_ascii_case("ogg")
            || cleaned.eq_ignore_ascii_case("mp3")
            || cleaned.eq_ignore_ascii_case("flac")
        {
            Self::Audio
        } else if cleaned.eq_ignore_ascii_case("mp4")
            || cleaned.eq_ignore_ascii_case("webm")
            || cleaned.eq_ignore_ascii_case("mkv")
        {
            Self::Video
        } else if cleaned.eq_ignore_ascii_case("zip")
            || cleaned.eq_ignore_ascii_case("tar")
            || cleaned.eq_ignore_ascii_case("7z")
            || cleaned.eq_ignore_ascii_case("pak")
        {
            Self::Archive
        } else if cleaned.eq_ignore_ascii_case("rs") {
            Self::CodeRust
        } else if cleaned.eq_ignore_ascii_case("json") {
            Self::CodeJson
        } else if cleaned.eq_ignore_ascii_case("toml")
            || cleaned.eq_ignore_ascii_case("yaml")
            || cleaned.eq_ignore_ascii_case("ini")
        {
            Self::CodeToml
        } else if cleaned.eq_ignore_ascii_case("rdc") || cleaned.eq_ignore_ascii_case("cap") {
            Self::CaptureRdc
        } else if cleaned.eq_ignore_ascii_case("log") || cleaned.eq_ignore_ascii_case("txt") {
            Self::Log
        } else if cleaned.eq_ignore_ascii_case("exe") || cleaned.eq_ignore_ascii_case("dll") {
            Self::Executable
        } else {
            Self::GenericFile
        }
    }

    /// 简短徽章标签（如 `"KTX2"`、`"GLTF"`、`"SPIRV"`、`"RDC"`）。
    #[must_use]
    pub const fn badge_label(self) -> &'static str {
        match self {
            Self::Folder => "DIR",
            Self::GenericFile => "FILE",
            Self::Image => "IMG",
            Self::TextureKtx2 => "KTX2",
            Self::TextureDds => "DDS",
            Self::MeshObj => "OBJ",
            Self::MeshFbx => "FBX",
            Self::MeshGltf => "GLTF",
            Self::ShaderHlsl => "HLSL",
            Self::ShaderGlsl => "GLSL",
            Self::ShaderSpirv => "SPV",
            Self::Material => "MAT",
            Self::Scene => "SCN",
            Self::Audio => "AUD",
            Self::Video => "VID",
            Self::Archive => "ZIP",
            Self::CodeRust => "RS",
            Self::CodeJson => "JSON",
            Self::CodeToml => "TOML",
            Self::CaptureRdc => "RDC",
            Self::Log => "LOG",
            Self::Executable => "EXE",
        }
    }

    /// 对应的基础矢量形状图标。
    #[must_use]
    pub const fn base_icon(self) -> MaterialIcon {
        match self {
            Self::Folder => MaterialIcon::Folder,
            Self::Image | Self::TextureKtx2 | Self::TextureDds => MaterialIcon::Image,
            Self::MeshObj | Self::MeshFbx | Self::MeshGltf | Self::Material => MaterialIcon::Cube,
            Self::Scene | Self::Archive => MaterialIcon::Layers,
            Self::ShaderHlsl
            | Self::ShaderGlsl
            | Self::ShaderSpirv
            | Self::CodeRust
            | Self::CodeJson
            | Self::CodeToml => MaterialIcon::Code,
            Self::CaptureRdc => MaterialIcon::Camera,
            Self::Executable => MaterialIcon::Terminal,
            Self::Audio | Self::Video => MaterialIcon::Play,
            Self::GenericFile | Self::Log => MaterialIcon::File,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_material_icon_parses_into_non_empty_subpaths() {
        for icon in MaterialIcon::ALL {
            let parsed = icon.parsed_path();
            assert!(
                !parsed.subpaths.is_empty(),
                "icon {:?} produced empty subpaths",
                icon
            );
        }
    }

    #[test]
    fn all_22_file_type_icons_resolve_extensions_and_badges() {
        assert_eq!(FileTypeIcon::ALL.len(), 22);
        assert_eq!(
            FileTypeIcon::from_extension(".ktx2"),
            FileTypeIcon::TextureKtx2
        );
        assert_eq!(FileTypeIcon::from_extension("GLB"), FileTypeIcon::MeshGltf);
        assert_eq!(
            FileTypeIcon::from_extension("spv"),
            FileTypeIcon::ShaderSpirv
        );
        assert_eq!(
            FileTypeIcon::from_extension("rdc"),
            FileTypeIcon::CaptureRdc
        );
        for ft in FileTypeIcon::ALL {
            assert!(!ft.badge_label().is_empty());
            assert!(!ft.base_icon().parsed_path().subpaths.is_empty());
        }
    }
}
