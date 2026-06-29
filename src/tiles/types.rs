macro_rules! generate_zoom_level {
    { $( $name:ident => $val:literal, )+ } => {
        #[derive(Eq, Hash, PartialEq, Clone, Copy, Debug)]
        pub enum ZoomLevel {
            $( $name = $val, )+
        }

        impl ZoomLevel {
            pub fn to_u8(&self) -> u8 {
                *self as u8
            }
        }

        impl TryFrom<u8> for ZoomLevel {
            type Error = ();

            fn try_from(v: u8) -> Result<Self, Self::Error> {
                match v {
                    $( $val => Ok(Self::$name), )+
                    _ => Err(()),
                }
            }
        }
    };
}

generate_zoom_level! {
    L0 => 0, L1 => 1, L2 => 2, L3 => 3, L4 => 4, L5 => 5,
    L6 => 6, L7 => 7, L8 => 8, L9 => 9, L10 => 10, L11 => 11,
    L12 => 12, L13 => 13, L14 => 14, L15 => 15, L16 => 16,
    L17 => 17, L18 => 18, L19 => 19, L20 => 20, L21 => 21,
    L22 => 22, L23 => 23, L24 => 24, L25 => 25,
}

/// Tile pixel size variants.
#[derive(Eq, Hash, PartialEq, Clone, Copy, Debug)]
pub enum TileSize {
    Normal,
    Large,
    VeryLarge,
}

impl TileSize {
    pub fn new(tile_pixels: u32) -> TileSize {
        match tile_pixels {
            768 => TileSize::VeryLarge,
            512 => TileSize::Large,
            _ => TileSize::Normal,
        }
    }

    pub fn to_pixels(&self) -> u32 {
        match self {
            TileSize::Normal => 256,
            TileSize::Large => 512,
            TileSize::VeryLarge => 768,
        }
    }

    pub fn get_url_postfix(&self) -> String {
        match self {
            TileSize::Normal => "".into(),
            TileSize::Large => "@2x".into(),
            TileSize::VeryLarge => "@3x".into(),
        }
    }

    pub fn url_postfix(&self) -> &'static str {
        match self {
            TileSize::Normal => "",
            TileSize::Large => "@2x",
            TileSize::VeryLarge => "@3x",
        }
    }
}

/// Image format for tile requests and storage.
#[derive(Eq, Hash, PartialEq, Clone, Copy, Debug)]
pub enum TileFormat {
    Png,
    Jpg,
    Webp,
}

impl Default for TileFormat {
    fn default() -> Self {
        TileFormat::Png
    }
}

impl TileFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            TileFormat::Png => "png",
            TileFormat::Jpg => "jpg",
            TileFormat::Webp => "webp",
        }
    }

    pub fn accept_mime(&self) -> &'static str {
        match self {
            TileFormat::Png => "image/png",
            TileFormat::Jpg => "image/jpeg",
            TileFormat::Webp => "image/webp",
        }
    }
}

/// Number of tiles away from center to fetch.
#[derive(Debug, Clone, Copy)]
pub struct Radius(pub u8);

/// Unique key identifying a tile download task.
#[derive(Eq, PartialEq, Hash, Clone, Debug)]
pub struct TileKey {
    pub x: u32,
    pub y: u32,
    pub zoom: u8,
    pub tile_size: TileSize,
    pub tile_format: TileFormat,
}

/// Download priority for scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DownloadPriority {
    Near = 0,
    Mid = 1,
    Far = 2,
    Elevation = 3,
}

pub enum UseCache {
    Yes,
    No,
}

impl UseCache {
    pub fn new(value: bool) -> UseCache {
        match value {
            true => UseCache::Yes,
            _ => UseCache::No,
        }
    }
}

pub enum AlreadyDownloaded {
    Yes,
    No,
}

impl AlreadyDownloaded {
    pub fn new(value: bool) -> AlreadyDownloaded {
        match value {
            true => AlreadyDownloaded::Yes,
            _ => AlreadyDownloaded::No,
        }
    }
}

pub enum FileExists {
    Yes,
    No,
}

impl FileExists {
    pub fn new(value: bool) -> FileExists {
        match value {
            true => FileExists::Yes,
            _ => FileExists::No,
        }
    }
}

#[derive(Debug)]
pub enum DownloadStatus {
    Downloading,
    Downloaded,
}
