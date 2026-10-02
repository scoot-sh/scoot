//! The PulseAudio native protocol, as much of it as the volume module
//! needs: framing, the tagstruct values it reads and writes, and the
//! replies it parses. Pure and dependency-free (`std` only), so the fuzz
//! crate compiles this file unchanged (`crates/scootbar/fuzz`).
//!
//! The wire was mapped against pipewire-pulse 1.6.8 on the Asahi machine
//! (a logging proxy around real `pactl` traffic, in the volume ticket's
//! record) and checked against the PulseAudio 17.0 source (fetched as
//! `nixpkgs#pulseaudio.src`): the frame is one `pstream` descriptor and a
//! tagstruct body, every multi-byte integer big-endian, every value with
//! its one-byte marker. What is parsed here is the prefix each reply is
//! read for; the rest (a sink's proplist, its ports) is never walked, so
//! a longer reply from a newer server still parses.

/// The volume of full scale: `pa_volume_t` `PA_VOLUME_NORM`.
pub const NORM: u32 = 65536;
/// No sink, source or card: `PA_INVALID_INDEX`.
pub const INVALID_INDEX: u32 = 0xffff_ffff;
/// The most bytes of one frame kept: the server's sink replies are a few
/// kilobytes (4 KB with a long proplist on the machine mapped); past this
/// the connection is dropped, so a hostile server cannot grow the read
/// buffer without limit. PulseAudio itself allows 16 MiB.
pub const MAX_FRAME: usize = 64 * 1024;
/// The most bytes kept of a name or description from the server: sink
/// names are short (`alsa_output...`, forty-odd bytes); more is cut at a
/// character boundary. Text from outside is untrusted, however same-user
/// the socket is.
pub const MAX_NAME: usize = 128;
/// Channels read of a volume at most: `PA_CHANNELS_MAX` is 32.
pub const MAX_CHANNELS: usize = 32;
/// The protocol version offered at `AUTH`: what `pactl` 17 speaks, without
/// the shared-memory flags (this client moves no audio, only control).
pub const PROTOCOL_VERSION: u32 = 35;

// Commands (`protocol-native.h`): only the ones sent or read.
pub const CMD_ERROR: u32 = 0;
pub const CMD_REPLY: u32 = 2;
pub const CMD_AUTH: u32 = 8;
pub const CMD_SET_CLIENT_NAME: u32 = 9;
pub const CMD_GET_SERVER_INFO: u32 = 20;
pub const CMD_GET_SINK_INFO: u32 = 21;
pub const CMD_GET_SOURCE_INFO: u32 = 23;
pub const CMD_SUBSCRIBE: u32 = 35;
pub const CMD_SET_SINK_VOLUME: u32 = 36;
pub const CMD_SET_SOURCE_VOLUME: u32 = 38;
pub const CMD_SET_SINK_MUTE: u32 = 39;
pub const CMD_SET_SOURCE_MUTE: u32 = 40;
pub const CMD_SUBSCRIBE_EVENT: u32 = 66;

// Value markers (`tagstruct.h`).
const MARK_STR: u8 = b't';
const MARK_STR_NULL: u8 = b'N';
const MARK_U32: u8 = b'L';
const MARK_SAMPLE_SPEC: u8 = b'a';
const MARK_ARBITRARY: u8 = b'x';
const MARK_BOOL_TRUE: u8 = b'1';
const MARK_BOOL_FALSE: u8 = b'0';
const MARK_CHANNEL_MAP: u8 = b'm';
const MARK_CVOLUME: u8 = b'v';
const MARK_PROPLIST: u8 = b'P';

// Subscription facilities and change kinds (`def.h`).
pub const MASK_SINK: u32 = 0x01;
pub const MASK_SOURCE: u32 = 0x02;
pub const MASK_SERVER: u32 = 0x80;
pub const FACILITY_SINK: u32 = 0x00;
pub const FACILITY_SOURCE: u32 = 0x01;
pub const FACILITY_SERVER: u32 = 0x07;
const TYPE_NEW: u32 = 0x00;
const TYPE_CHANGE: u32 = 0x10;
const TYPE_REMOVE: u32 = 0x20;

/// A change the server reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    New,
    Changed,
    Removed,
}

/// What a sink or source is (`Sink` subscribes and sets with the sink
/// commands, `Source` with the source ones; the replies parse the same).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sink,
    Source,
}

impl Kind {
    pub fn get_info(self) -> u32 {
        match self {
            Self::Sink => CMD_GET_SINK_INFO,
            Self::Source => CMD_GET_SOURCE_INFO,
        }
    }

    pub fn set_volume(self) -> u32 {
        match self {
            Self::Sink => CMD_SET_SINK_VOLUME,
            Self::Source => CMD_SET_SOURCE_VOLUME,
        }
    }

    pub fn set_mute(self) -> u32 {
        match self {
            Self::Sink => CMD_SET_SINK_MUTE,
            Self::Source => CMD_SET_SOURCE_MUTE,
        }
    }

    pub fn subscribe_mask(self) -> u32 {
        match self {
            Self::Sink => MASK_SINK | MASK_SERVER,
            Self::Source => MASK_SOURCE | MASK_SERVER,
        }
    }

    /// Whether a subscription event is about this kind (the server's own
    /// changes are reported to both).
    pub fn owns(self, facility: u32) -> bool {
        match self {
            Self::Sink => facility == FACILITY_SINK || facility == FACILITY_SERVER,
            Self::Source => facility == FACILITY_SOURCE || facility == FACILITY_SERVER,
        }
    }
}

/// One frame on the wire: the command, the tag and what follows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub cmd: u32,
    pub tag: u32,
    pub payload: &'a [u8],
}

/// The frame starting `buf`, if it has all arrived: the frame and how many
/// bytes it takes. `None` is "wait for more"; `Err` is a length no server
/// may send (past [`MAX_FRAME`], or shorter than a command and a tag), and
/// the connection is dropped on it.
pub fn frame_at(buf: &[u8]) -> Result<Option<(Frame<'_>, usize)>, ()> {
    if buf.len() < 20 {
        return Ok(None);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if !(10..=MAX_FRAME).contains(&len) {
        return Err(());
    }
    if buf.len() < 20 + len {
        return Ok(None);
    }
    let body = &buf[20..20 + len];
    if body[0] != MARK_U32 || body[5] != MARK_U32 {
        return Err(());
    }
    let cmd = u32::from_be_bytes([body[1], body[2], body[3], body[4]]);
    let tag = u32::from_be_bytes([body[6], body[7], body[8], body[9]]);
    Ok(Some((
        Frame {
            cmd,
            tag,
            payload: &body[10..],
        },
        20 + len,
    )))
}

/// Writes one packet for `cmd` and `tag` around `payload` into `out`:
/// the `pstream` descriptor (length, channel -1, flags and offset zero)
/// and the command and tag as tagged values. `None` when it does not fit.
pub fn encode_into(cmd: u32, tag: u32, payload: &[u8], out: &mut [u8]) -> Option<usize> {
    let len = 10 + payload.len();
    if len > MAX_FRAME || out.len() < 20 + len {
        return None;
    }
    out[0..4].copy_from_slice(&(len as u32).to_be_bytes());
    out[4..8].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
    out[8..20].fill(0);
    out[20] = MARK_U32;
    out[21..25].copy_from_slice(&cmd.to_be_bytes());
    out[25] = MARK_U32;
    out[26..30].copy_from_slice(&tag.to_be_bytes());
    out[30..30 + payload.len()].copy_from_slice(payload);
    Some(20 + len)
}

/// Reads tagged values off a frame's payload, bounds-checked. `None` is a
/// malformed reply, and the connection is dropped on it: every getter
/// checks its marker and its length first, so no input panics and none is
/// trusted past what is there.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(payload: &'a [u8]) -> Self {
        Self {
            buf: payload,
            pos: 0,
        }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let taken = &self.buf[self.pos..end];
        self.pos = end;
        Some(taken)
    }

    fn marker(&mut self) -> Option<u8> {
        self.take(1).map(|taken| taken[0])
    }

    pub fn get_u32(&mut self) -> Option<u32> {
        if self.marker()? != MARK_U32 {
            return None;
        }
        let taken = self.take(4)?;
        Some(u32::from_be_bytes([taken[0], taken[1], taken[2], taken[3]]))
    }

    pub fn get_bool(&mut self) -> Option<bool> {
        match self.marker()? {
            MARK_BOOL_TRUE => Some(true),
            MARK_BOOL_FALSE => Some(false),
            _ => None,
        }
    }

    /// A string, or `None` for an explicit null. Cut at [`MAX_NAME`] bytes
    /// on a character boundary; without a terminator in what is left it is
    /// malformed.
    pub fn get_str<'b>(&mut self, into: &'b mut Bounded) -> Option<Option<&'b str>> {
        match self.marker()? {
            MARK_STR_NULL => {
                into.clear();
                Some(None)
            }
            MARK_STR => {
                let rest = &self.buf[self.pos..];
                let end = rest.iter().position(|byte| *byte == 0)?;
                let text = &rest[..end];
                into.set_bytes(text);
                self.pos += end + 1;
                Some(Some(into.as_str()))
            }
            _ => None,
        }
    }

    /// A channel volume: at most [`MAX_CHANNELS`] channels, each a raw
    /// `pa_volume_t`.
    pub fn get_cvolume(&mut self, volumes: &mut [u32; MAX_CHANNELS]) -> Option<usize> {
        if self.marker()? != MARK_CVOLUME {
            return None;
        }
        let channels = self.take(1).map(|taken| taken[0] as usize)?;
        if channels == 0 || channels > MAX_CHANNELS {
            return None;
        }
        for volume in volumes.iter_mut().take(channels) {
            let taken = self.take(4)?;
            *volume = u32::from_be_bytes([taken[0], taken[1], taken[2], taken[3]]);
        }
        Some(channels)
    }

    /// Skips a sample specification (format, channels, rate): the module
    /// never shows it.
    pub fn skip_spec(&mut self) -> Option<()> {
        if self.marker()? != MARK_SAMPLE_SPEC {
            return None;
        }
        self.take(2 + 4)?;
        Some(())
    }

    /// Skips a channel map (channels, positions).
    pub fn skip_map(&mut self) -> Option<()> {
        if self.marker()? != MARK_CHANNEL_MAP {
            return None;
        }
        let channels = self.take(1).map(|taken| taken[0] as usize)?;
        if channels > MAX_CHANNELS {
            return None;
        }
        self.take(channels)?;
        Some(())
    }

    /// Skips a sample specification and a channel map (format, channels,
    /// rate; channels, positions): the module never shows them.
    pub fn skip_spec_map(&mut self) -> Option<()> {
        self.skip_spec()?;
        self.skip_map()
    }

    /// The next marker without consuming it: what follows an optional
    /// field.
    pub fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }
}

/// Writes tagged values into a fixed buffer: requests are tiny (an `AUTH`
/// with its cookie is under 300 bytes), so nothing allocates.
pub struct Writer {
    buf: [u8; 512],
    len: usize,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            buf: [0; 512],
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) -> Option<()> {
        *self.buf.get_mut(self.len)? = byte;
        self.len += 1;
        Some(())
    }

    fn extend(&mut self, bytes: &[u8]) -> Option<()> {
        if self.len + bytes.len() > self.buf.len() {
            return None;
        }
        self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Some(())
    }

    pub fn put_u32(&mut self, value: u32) -> Option<()> {
        self.push(MARK_U32)?;
        let bytes = value.to_be_bytes();
        self.extend(&bytes)
    }

    pub fn put_bool(&mut self, value: bool) -> Option<()> {
        self.push(if value {
            MARK_BOOL_TRUE
        } else {
            MARK_BOOL_FALSE
        })
    }

    pub fn put_str(&mut self, text: &str) -> Option<()> {
        if text.len() > MAX_NAME {
            return None;
        }
        self.push(MARK_STR)?;
        self.extend(text.as_bytes())?;
        self.push(0)
    }

    pub fn put_arbitrary(&mut self, bytes: &[u8]) -> Option<()> {
        if bytes.len() > MAX_FRAME {
            return None;
        }
        self.push(MARK_ARBITRARY)?;
        let len = (bytes.len() as u32).to_be_bytes();
        self.extend(&len)?;
        self.extend(bytes)
    }

    pub fn put_cvolume(&mut self, volumes: &[u32]) -> Option<()> {
        if volumes.is_empty() || volumes.len() > MAX_CHANNELS {
            return None;
        }
        self.push(MARK_CVOLUME)?;
        self.push(volumes.len() as u8)?;
        for volume in volumes {
            self.extend(&volume.to_be_bytes())?;
        }
        Some(())
    }

    pub fn put_proplist(&mut self, key: &str, value: &[u8]) -> Option<()> {
        self.push(MARK_PROPLIST)?;
        self.put_str(key)?;
        self.put_u32(value.len() as u32)?;
        self.put_arbitrary(value)
    }

    pub fn done(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// The `AUTH` payload: the protocol version and the cookie read from the
/// user's cookie file (256 bytes). `None` is a cookie of the wrong size
/// (never from the file, only from a test).
pub fn auth_payload(cookie: &[u8], out: &mut Writer) -> Option<()> {
    out.put_u32(PROTOCOL_VERSION)?;
    if cookie.len() != 256 {
        return None;
    }
    out.put_arbitrary(cookie)
}

/// The client's name: a proplist with only `application.name`, then its
/// null terminator.
pub fn name_payload(out: &mut Writer) -> Option<()> {
    out.put_proplist("application.name", b"scootbar\0")?;
    out.push(MARK_STR_NULL)
}

/// What `GET_SERVER_INFO` answers that the module keeps: the default sink
/// and source names. Newer fields (the channel map, and whatever a later
/// server appends) are not read, so a longer reply still parses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDefaults {
    pub sink: Bounded,
    pub source: Bounded,
}

pub fn parse_server_info(payload: &[u8]) -> Option<ServerDefaults> {
    let mut reader = Reader::new(payload);
    // The server name, version, user and host: read and dropped.
    let mut scratch = Bounded::empty();
    for _ in 0..4 {
        reader.get_str(&mut scratch)?;
    }
    reader.skip_spec()?;
    let mut sink = Bounded::empty();
    let mut source = Bounded::empty();
    // A missing default is an explicit null (no sink at all): kept empty.
    reader.get_str(&mut sink)?;
    reader.get_str(&mut source)?;
    // The cookie, then the channel map: whatever a later server appends
    // after is not read, so a longer reply still parses.
    reader.get_u32()?;
    if reader.peek() == Some(MARK_CHANNEL_MAP) {
        reader.skip_map()?;
    }
    Some(ServerDefaults { sink, source })
}

/// What one `GET_SINK_INFO` (or source) reply answers that the module
/// keeps: the index, the name and description, the loudest channel and
/// whether it is muted. The tail (monitor, latency, flags, proplist,
/// ports) is never walked: parsing stops at mute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: Bounded,
    pub description: Bounded,
    pub channels: usize,
    pub volumes: [u32; MAX_CHANNELS],
    pub muted: bool,
}

impl DeviceInfo {
    /// The loudest channel: what the bar shows and sets from.
    pub fn level(&self) -> u32 {
        self.volumes[..self.channels]
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
    }
}

pub fn parse_device_info(payload: &[u8]) -> Option<DeviceInfo> {
    let mut reader = Reader::new(payload);
    let index = reader.get_u32()?;
    let mut name = Bounded::empty();
    let mut description = Bounded::empty();
    // A null name is malformed (the read itself erroring returns above).
    reader.get_str(&mut name)??;
    // An empty description is shown as the name; a null one is missing,
    // and the name stands in for it too.
    match reader.get_str(&mut description)? {
        Some(text) if !text.is_empty() => {}
        _ => {
            description.clear();
        }
    }
    reader.skip_spec_map()?;
    // The owning module: read and dropped.
    reader.get_u32()?;
    let mut volumes = [0; MAX_CHANNELS];
    let channels = reader.get_cvolume(&mut volumes)?;
    let muted = reader.get_bool()?;
    Some(DeviceInfo {
        index,
        name,
        description,
        channels,
        volumes,
        muted,
    })
}

/// A subscription event: what kind of thing changed, how, and which index
/// (`INVALID_INDEX` for the server itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub facility: u32,
    pub change: Change,
    pub index: u32,
}

pub fn parse_event(payload: &[u8]) -> Option<Event> {
    let mut reader = Reader::new(payload);
    let event = reader.get_u32()?;
    let index = reader.get_u32()?;
    let change = match event & 0xf0 {
        TYPE_NEW => Change::New,
        TYPE_CHANGE => Change::Changed,
        TYPE_REMOVE => Change::Removed,
        _ => return None,
    };
    let facility = event & 0x0f;
    match facility {
        FACILITY_SINK | FACILITY_SOURCE | FACILITY_SERVER => Some(Event {
            facility,
            change,
            index,
        }),
        _ => None,
    }
}

/// Copies `text` into fixed bytes, cut at a character boundary.
fn store_bounded(into: &mut [u8; MAX_NAME], len: &mut usize, text: &[u8]) {
    let room = text.len().min(MAX_NAME);
    into[..room].copy_from_slice(&text[..room]);
    let mut cut = room;
    while cut > 0 && std::str::from_utf8(&into[..cut]).is_err() {
        cut -= 1;
    }
    *len = cut;
}

/// A bounded name or description from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounded {
    bytes: [u8; MAX_NAME],
    len: usize,
}

impl Bounded {
    pub fn empty() -> Self {
        Self {
            bytes: [0; MAX_NAME],
            len: 0,
        }
    }

    pub fn set_bytes(&mut self, text: &[u8]) {
        store_bounded(&mut self.bytes, &mut self.len, text);
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn set(&mut self, text: &str) {
        store_bounded(&mut self.bytes, &mut self.len, text.as_bytes());
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

/// A raw volume as whole percent of [`NORM`], rounded: 32113 is 49.
pub fn to_percent(raw: u32) -> u32 {
    ((raw as u64 * 100 + u64::from(NORM) / 2) / u64::from(NORM)) as u32
}

/// Whole percent back to raw, rounded and saturating: 49 is 32113.
pub fn from_percent(percent: u32) -> u32 {
    ((percent as u64 * u64::from(NORM) + 50) / 100).min(u64::from(u32::MAX)) as u32
}
