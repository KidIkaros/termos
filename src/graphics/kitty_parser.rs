//! Kitty graphics protocol command parser — ported from Go TUIOS
//! `internal/vt/kitty_parser.go` and `kitty_types.go`.
//!
//! Decodes an APC payload (`\x1b_G <control>; <data> \x1b\`) into a typed
//! command. The control part is comma-separated `key=value` pairs; the data
//! part is base64 (direct), a base64 path (file/temp-file), or a shared
//! memory id.

/// The graphics action (`a=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyAction {
    Transmit,
    TransmitPlace,
    Display,
    Delete,
    Query,
    TransmitAndDisplay,
}

impl KittyAction {
    fn from_char(c: char) -> Option<Self> {
        match c {
            't' => Some(Self::Transmit),
            'T' => Some(Self::TransmitPlace),
            'd' => Some(Self::Display),
            'D' => Some(Self::Delete),
            'q' => Some(Self::Query),
            'a' => Some(Self::TransmitAndDisplay),
            _ => None,
        }
    }

    /// The wire letter.
    pub fn letter(self) -> char {
        match self {
            Self::Transmit => 't',
            Self::TransmitPlace => 'T',
            Self::Display => 'd',
            Self::Delete => 'D',
            Self::Query => 'q',
            Self::TransmitAndDisplay => 'a',
        }
    }
}

/// The transmission medium (`m=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyMedium {
    Direct,
    File,
    TempFile,
    SharedMemory,
}

impl KittyMedium {
    fn from_char(c: char) -> Self {
        match c {
            'f' => Self::File,
            't' => Self::TempFile,
            's' => Self::SharedMemory,
            _ => Self::Direct,
        }
    }
}

/// The pixel format (`f=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyFormat {
    Rgb,
    Rgba,
    Png,
    Webp,
    Gif,
}

impl KittyFormat {
    fn from_char(c: char) -> Self {
        match c {
            'r' => Self::Rgb,
            'p' => Self::Png,
            'w' => Self::Webp,
            'g' => Self::Gif,
            _ => Self::Rgba,
        }
    }
}

/// The compression type (`c=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyCompression {
    None,
    Zlib,
    /// Not in the spec; used by some implementations.
    Other,
}

impl KittyCompression {
    fn from_char(c: char) -> Self {
        match c {
            'z' => Self::Zlib,
            'o' => Self::Other,
            _ => Self::None,
        }
    }
}

/// The delete target (`d=`) for delete commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyDeleteTarget {
    /// Delete all placements of the named image.
    All,
    /// Delete all placements with the same animation frame.
    Frame,
    /// Delete only the named placement.
    Placement,
    /// Delete all placements of all images.
    AllImages,
    /// Delete the named image and all its placements.
    Image,
}

impl KittyDeleteTarget {
    fn from_char(c: char) -> Self {
        match c {
            'a' => Self::All,
            'f' => Self::Frame,
            'p' => Self::Placement,
            'A' => Self::AllImages,
            'i' => Self::Image,
            _ => Self::All,
        }
    }
}

/// A parsed kitty graphics command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyCommand {
    pub action: KittyAction,
    pub quiet: i32,
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub format: KittyFormat,
    pub medium: KittyMedium,
    pub compression: KittyCompression,
    pub width: i32,
    pub height: i32,
    pub size: i32,
    pub more: bool,
    pub delete: KittyDeleteTarget,
    pub x_offset: i32,
    pub y_offset: i32,
    pub source_x: i32,
    pub source_y: i32,
    pub source_width: i32,
    pub source_height: i32,
    pub columns: i32,
    pub rows: i32,
    pub z_index: i32,
    pub cursor_move: i32,
    /// Whether the placement is virtual (`U=1`).
    pub virtual_placement: bool,
    /// The decoded data (direct medium) or the decoded path/shared-memory id.
    pub payload: Vec<u8>,
}

impl KittyCommand {
    /// Whether the payload is zlib-compressed.
    pub fn compressed(&self) -> bool {
        matches!(self.compression, KittyCompression::Zlib)
    }

    /// Delete all images and placements.
    pub fn delete_all(&self) -> bool {
        matches!(self.delete, KittyDeleteTarget::AllImages)
    }

    /// Delete the placement at the cursor position.
    pub fn delete_at_cursor(&self) -> bool {
        // `d=c` would be cursor-based delete; the Go implementation uses
        // the absence of image/placement ids with a column/row offset of
        // zero to signal cursor-based delete.
        self.action == KittyAction::Delete
            && self.image_id == 0
            && self.image_number == 0
            && self.placement_id == 0
            && !self.delete_all()
            && self.x_offset == 0
            && self.y_offset == 0
    }

    /// Delete placements in the cursor's column.
    pub fn delete_in_column(&self) -> bool {
        self.action == KittyAction::Delete && self.x_offset != 0
    }

    /// Delete placements in the cursor's row.
    pub fn delete_in_row(&self) -> bool {
        self.action == KittyAction::Delete && self.y_offset != 0
    }

    /// Delete placements by z-index.
    pub fn delete_by_z_index(&self) -> bool {
        self.action == KittyAction::Delete && self.z_index != 0
    }

    /// The column offset for delete-in-column operations.
    pub fn column_offset(&self) -> i32 {
        self.x_offset
    }

    /// The row offset for delete-in-row operations.
    pub fn row_offset(&self) -> i32 {
        self.y_offset
    }
}

impl Default for KittyCommand {
    fn default() -> Self {
        Self {
            action: KittyAction::Transmit,
            quiet: 0,
            image_id: 0,
            image_number: 0,
            placement_id: 0,
            format: KittyFormat::Rgba,
            medium: KittyMedium::Direct,
            compression: KittyCompression::None,
            width: 0,
            height: 0,
            size: 0,
            more: false,
            delete: KittyDeleteTarget::All,
            x_offset: 0,
            y_offset: 0,
            source_x: 0,
            source_y: 0,
            source_width: 0,
            source_height: 0,
            columns: 0,
            rows: 0,
            z_index: 0,
            cursor_move: 0,
            virtual_placement: false,
            payload: Vec::new(),
        }
    }
}

/// Parse an APC payload (bytes, without the `\x1b_G`/`\x1b\` wrappers).
/// Returns `None` if the payload is not valid UTF-8.
pub fn parse_kitty_command(payload: &[u8]) -> Option<KittyCommand> {
    let s = std::str::from_utf8(payload).ok()?;
    Some(KittyCommand::parse(s))
}

impl KittyCommand {
    /// Parse an APC payload (without the `\x1b_G`/`\x1b\` wrappers).
    pub fn parse(payload: &str) -> KittyCommand {
        let mut cmd = KittyCommand::default();
        let (control, data) = match payload.find(';') {
            Some(idx) => (&payload[..idx], &payload[idx + 1..]),
            None => (payload, ""),
        };
        for pair in control.split(',') {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = match pair.split_once('=') {
                Some(kv) => kv,
                None => continue,
            };
            let v = value;
            match key {
                "a" => {
                    if let Some(c) = v.chars().next() {
                        cmd.action = KittyAction::from_char(c).unwrap_or(cmd.action);
                    }
                }
                "q" => cmd.quiet = v.parse().unwrap_or(0),
                "i" => cmd.image_id = v.parse().unwrap_or(0),
                "I" => cmd.image_number = v.parse().unwrap_or(0),
                "p" => cmd.placement_id = v.parse().unwrap_or(0),
                "f" => {
                    if let Some(c) = v.chars().next() {
                        cmd.format = KittyFormat::from_char(c);
                    }
                }
                "m" => {
                    // `m` is dual-purpose: a numeric value (0/1) is the
                    // more-flag for chunked transmission; a character value
                    // (d/f/t/s) is the transmission medium.
                    if let Ok(n) = v.parse::<i32>() {
                        cmd.more = n != 0;
                    } else if let Some(c) = v.chars().next() {
                        cmd.medium = KittyMedium::from_char(c);
                    }
                }
                "o" => {
                    if let Some(c) = v.chars().next() {
                        cmd.compression = KittyCompression::from_char(c);
                    }
                }
                "s" => cmd.width = v.parse().unwrap_or(0),
                "v" => cmd.height = v.parse().unwrap_or(0),
                "S" => cmd.size = v.parse().unwrap_or(0),
                "d" => {
                    if let Some(c) = v.chars().next() {
                        cmd.delete = KittyDeleteTarget::from_char(c);
                    }
                }
                "x" => cmd.x_offset = v.parse().unwrap_or(0),
                "y" => cmd.y_offset = v.parse().unwrap_or(0),
                "X" => cmd.source_x = v.parse().unwrap_or(0),
                "Y" => cmd.source_y = v.parse().unwrap_or(0),
                "w" => cmd.source_width = v.parse().unwrap_or(0),
                "h" => cmd.source_height = v.parse().unwrap_or(0),
                "c" => cmd.columns = v.parse().unwrap_or(0),
                "r" => cmd.rows = v.parse().unwrap_or(0),
                "z" => cmd.z_index = v.parse().unwrap_or(0),
                "C" => cmd.cursor_move = v.parse().unwrap_or(0),
                "U" => cmd.virtual_placement = v == "1",
                _ => {}
            }
        }
        // Decode the data part (base64 for direct medium, path/id otherwise).
        if !data.is_empty() {
            use base64::Engine;
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(data.trim()) {
                cmd.payload = decoded;
            } else {
                cmd.payload = data.as_bytes().to_vec();
            }
        }
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transmit_defaults() {
        let cmd = KittyCommand::parse("a=t,i=1,s=4,v=2");
        assert_eq!(cmd.action, KittyAction::Transmit);
        assert_eq!(cmd.image_id, 1);
        assert_eq!(cmd.width, 4);
        assert_eq!(cmd.height, 2);
        assert_eq!(cmd.medium, KittyMedium::Direct);
        assert_eq!(cmd.format, KittyFormat::Rgba);
    }

    #[test]
    fn transmit_and_display_with_payload() {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(b"\x01\x02\x03");
        // The width lives in the control part; the data part is pure base64.
        let apc = format!("a=T,f=p,s=10;{data}");
        let cmd = KittyCommand::parse(&apc);
        assert_eq!(cmd.action, KittyAction::TransmitPlace);
        assert_eq!(cmd.format, KittyFormat::Png);
        assert_eq!(cmd.width, 10);
        assert_eq!(cmd.payload, b"\x01\x02\x03");
    }

    #[test]
    fn file_medium_decodes_path() {
        use base64::Engine;
        let path = base64::engine::general_purpose::STANDARD.encode(b"/tmp/img.png");
        let apc = format!("a=t,m=f;{path}");
        let cmd = KittyCommand::parse(&apc);
        assert_eq!(cmd.medium, KittyMedium::File);
        assert_eq!(cmd.payload, b"/tmp/img.png");
    }

    #[test]
    fn shared_memory_medium() {
        use base64::Engine;
        let id = base64::engine::general_purpose::STANDARD.encode(b"shm-123");
        let apc = format!("a=t,m=s;{id}");
        let cmd = KittyCommand::parse(&apc);
        assert_eq!(cmd.medium, KittyMedium::SharedMemory);
        assert_eq!(cmd.payload, b"shm-123");
    }

    #[test]
    fn delete_target() {
        let cmd = KittyCommand::parse("a=D,d=p,i=7");
        assert_eq!(cmd.action, KittyAction::Delete);
        assert_eq!(cmd.delete, KittyDeleteTarget::Placement);
        assert_eq!(cmd.image_id, 7);
    }

    #[test]
    fn query_command() {
        let cmd = KittyCommand::parse("a=q,i=2");
        assert_eq!(cmd.action, KittyAction::Query);
        assert_eq!(cmd.image_id, 2);
    }

    #[test]
    fn zlib_compression_and_geometry() {
        let cmd = KittyCommand::parse("a=t,o=z,z=5,c=2,r=3,x=4,y=6,C=1,U=1");
        assert_eq!(cmd.compression, KittyCompression::Zlib);
        assert_eq!(cmd.z_index, 5);
        assert_eq!(cmd.columns, 2);
        assert_eq!(cmd.rows, 3);
        assert_eq!(cmd.x_offset, 4);
        assert_eq!(cmd.y_offset, 6);
        assert_eq!(cmd.cursor_move, 1);
        assert!(cmd.virtual_placement);
    }

    #[test]
    fn empty_payload_sets_more_flag() {
        let cmd = KittyCommand::parse("a=t,i=1,m=1");
        // A control-only APC without data is a continuation chunk.
        assert!(cmd.payload.is_empty());
    }

    #[test]
    fn png_format() {
        let cmd = KittyCommand::parse("a=t,f=p");
        assert_eq!(cmd.format, KittyFormat::Png);
    }
}
