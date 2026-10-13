//! `layout.bin` (the plan's A3): where the host's inputs and outputs are
//! among the program's leaves. 35 u16 little-endian fields in this order,
//! zero padding to 72, then the interface hash; leaves are absolute (input
//! leaves for `in_*`/`ev_*`, output leaves for `out_*`/`pub_*`, a fix's own
//! for `fix_*`), [`NONE`] for none. `engine/profile` writes it.
//!
//! Version 2 appends the list table, for an engine with lists in its state
//! ([`StateLists`]): `count u16, gps_list u16`, then `count` entries
//! `{list u16, leaf u16, max_items u16, item_leaves u16}`. Version 1 is
//! the fixed part alone: no state lists, `gps` list 0.

/// No leaf.
pub const NONE: u16 = 0xFFFF;
/// The newest version this crate reads (it reads 1 too).
pub const VERSION: u16 = 2;
/// `layout.bin`'s fixed part.
pub const BYTES: usize = 80;
/// The list table's header and each entry.
pub const LISTS_HEADER_BYTES: usize = 4;
pub const LIST_ENTRY_BYTES: usize = 8;
const HASH_AT: usize = 72;
const FIELDS: usize = 35;

/// The fields' names, in order (for refusals that name one).
pub(crate) const FIELD_NAMES: [&str; FIELDS] = [
    "version", "input_count", "output_count", "list_count", "in_state", "state_leaves", "in_event", "event_leaves",
    "in_timestamp", "in_gps", "out_state", "out_view", "view_leaves", "out_wake", "pub_base", "pub_leaves", "ev_gps",
    "ev_gps_fix", "ev_gps_fix_t", "ev_gps_fix_lat", "ev_gps_fix_lon", "ev_gps_fix_alt", "ev_gps_velocity",
    "ev_gps_velocity_speed", "ev_gps_velocity_heading", "ev_wake", "fix_leaves", "fix_t", "fix_lat", "fix_lon",
    "fix_alt", "gps_max_items", "pub_max_bytes", "event_max_bytes", "state_free_strings",
];

/// Why bytes aren't a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not [`BYTES`] long (version 1), or not the fixed part and the list
    /// table its count says (version 2).
    Size,
    /// Another version.
    Version,
    /// The padding isn't zero.
    Padding,
    /// A version 2 table without a state list.
    Lists,
}

/// One list in the state, from the list table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateList {
    /// The program's list id.
    pub list: u16,
    /// Its handle's leaf in the `state` input (absolute); the output's is
    /// as far into the output's `state`.
    pub leaf: u16,
    /// The input list's bound, and the leaves of an item.
    pub max_items: u16,
    pub item_leaves: u16,
}

impl StateList {
    /// The slots its items take, at most.
    pub fn slots(&self) -> u32 {
        self.max_items as u32 * self.item_leaves as u32
    }
}

/// `layout.bin`'s list table, read in place: `gps`'s list id and the
/// lists in the state, in table order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateLists<'a> {
    pub gps_list: u16,
    /// The entries' bytes.
    entries: &'a [u8],
}

impl<'a> StateLists<'a> {
    /// The table of `b`, a layout [`Layout::parse`] accepted.
    pub fn parse(b: &'a [u8]) -> Result<StateLists<'a>, Error> {
        let Some(table) = b.get(BYTES..).filter(|t| !t.is_empty()) else {
            return Ok(StateLists { gps_list: 0, entries: &[] });
        };
        let at = |i: usize| table.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]])).ok_or(Error::Size);
        let (count, gps_list) = (at(0)? as usize, at(2)?);
        if table.len() != LISTS_HEADER_BYTES + count * LIST_ENTRY_BYTES {
            return Err(Error::Size);
        }
        Ok(StateLists { gps_list, entries: &table[LISTS_HEADER_BYTES..] })
    }

    pub fn len(&self) -> usize {
        self.entries.len() / LIST_ENTRY_BYTES
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry `k`.
    pub fn get(&self, k: usize) -> Option<StateList> {
        let e = self.entries.get(k * LIST_ENTRY_BYTES..(k + 1) * LIST_ENTRY_BYTES)?;
        let at = |i: usize| u16::from_le_bytes([e[i], e[i + 1]]);
        Some(StateList { list: at(0), leaf: at(2), max_items: at(4), item_leaves: at(6) })
    }

    pub fn iter(&self) -> impl Iterator<Item = StateList> + '_ {
        (0..self.len()).filter_map(|k| self.get(k))
    }

    /// The entry of list id `list`, and its place in the table.
    pub fn of_list(&self, list: u16) -> Option<(usize, StateList)> {
        self.iter().enumerate().find(|(_, l)| l.list == list)
    }

    /// Where entry `k`'s items start in a host's buffer that holds every
    /// state list's at its most, in table order.
    pub fn offset(&self, k: usize) -> u32 {
        self.iter().take(k).map(|l| l.slots()).sum()
    }

    /// The slots every state list takes at its most.
    pub fn slots(&self) -> u32 {
        self.iter().map(|l| l.slots()).sum()
    }
}

/// A parsed `layout.bin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub version: u16,
    /// The program's input leaves.
    pub input_count: u16,
    /// The program's output leaves.
    pub output_count: u16,
    /// The program's input lists: `gps` and the state's ([`StateLists`]).
    pub list_count: u16,
    /// The `state` input's presence leaf; its fields follow.
    pub in_state: u16,
    /// The state's leaves (without the presence leaf).
    pub state_leaves: u16,
    /// The `event` input's first leaf.
    pub in_event: u16,
    pub event_leaves: u16,
    pub in_timestamp: u16,
    /// The `gps` input's leaf (a list handle).
    pub in_gps: u16,
    /// The output's `state`, `view` (or [`NONE`]) and `wake`.
    pub out_state: u16,
    pub out_view: u16,
    pub view_leaves: u16,
    pub out_wake: u16,
    /// The published value (the view, or the state without one): its first
    /// output leaf and its leaves.
    pub pub_base: u16,
    pub pub_leaves: u16,
    /// The built-in events' input leaves: `gps` and its parts (presence
    /// leaves for the variant and the optional records), and `wake`.
    pub ev_gps: u16,
    pub ev_gps_fix: u16,
    pub ev_gps_fix_t: u16,
    pub ev_gps_fix_lat: u16,
    pub ev_gps_fix_lon: u16,
    pub ev_gps_fix_alt: u16,
    pub ev_gps_velocity: u16,
    pub ev_gps_velocity_speed: u16,
    pub ev_gps_velocity_heading: u16,
    pub ev_wake: u16,
    /// A fix in `gps`: its leaves, and each field's leaf within it.
    pub fix_leaves: u16,
    pub fix_t: u16,
    pub fix_lat: u16,
    pub fix_lon: u16,
    pub fix_alt: u16,
    pub gps_max_items: u16,
    /// The largest bitwire encodings of the published value and an author's
    /// event, bytes.
    pub pub_max_bytes: u16,
    pub event_max_bytes: u16,
    /// The state's string leaves that aren't enums (a host refuses any).
    pub state_free_strings: u16,
    pub interface_hash: [u8; 8],
}

impl Layout {
    /// The fixed part of `b`, a whole `layout.bin` (its list table is
    /// checked for size here, read by [`StateLists::parse`]).
    pub fn parse(b: &[u8]) -> Result<Layout, Error> {
        if b.len() < BYTES {
            return Err(Error::Size);
        }
        let f: [u16; FIELDS] = core::array::from_fn(|i| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]));
        match f[0] {
            1 if b.len() != BYTES => return Err(Error::Size),
            1 => {}
            2 if b.len() == BYTES => return Err(Error::Size),
            2 if StateLists::parse(b)?.is_empty() => return Err(Error::Lists),
            2 => {}
            _ => return Err(Error::Version),
        }
        if b[2 * FIELDS..HASH_AT].iter().any(|&x| x != 0) {
            return Err(Error::Padding);
        }
        let mut interface_hash = [0; 8];
        interface_hash.copy_from_slice(&b[HASH_AT..BYTES]);
        let [
            version,
            input_count,
            output_count,
            list_count,
            in_state,
            state_leaves,
            in_event,
            event_leaves,
            in_timestamp,
            in_gps,
            out_state,
            out_view,
            view_leaves,
            out_wake,
            pub_base,
            pub_leaves,
            ev_gps,
            ev_gps_fix,
            ev_gps_fix_t,
            ev_gps_fix_lat,
            ev_gps_fix_lon,
            ev_gps_fix_alt,
            ev_gps_velocity,
            ev_gps_velocity_speed,
            ev_gps_velocity_heading,
            ev_wake,
            fix_leaves,
            fix_t,
            fix_lat,
            fix_lon,
            fix_alt,
            gps_max_items,
            pub_max_bytes,
            event_max_bytes,
            state_free_strings,
        ] = f;
        Ok(Layout {
            version,
            input_count,
            output_count,
            list_count,
            in_state,
            state_leaves,
            in_event,
            event_leaves,
            in_timestamp,
            in_gps,
            out_state,
            out_view,
            view_leaves,
            out_wake,
            pub_base,
            pub_leaves,
            ev_gps,
            ev_gps_fix,
            ev_gps_fix_t,
            ev_gps_fix_lat,
            ev_gps_fix_lon,
            ev_gps_fix_alt,
            ev_gps_velocity,
            ev_gps_velocity_speed,
            ev_gps_velocity_heading,
            ev_wake,
            fix_leaves,
            fix_t,
            fix_lat,
            fix_lon,
            fix_alt,
            gps_max_items,
            pub_max_bytes,
            event_max_bytes,
            state_free_strings,
            interface_hash,
        })
    }

    /// The fields in order (the bytes' order).
    fn fields(&self) -> [u16; FIELDS] {
        [
            self.version,
            self.input_count,
            self.output_count,
            self.list_count,
            self.in_state,
            self.state_leaves,
            self.in_event,
            self.event_leaves,
            self.in_timestamp,
            self.in_gps,
            self.out_state,
            self.out_view,
            self.view_leaves,
            self.out_wake,
            self.pub_base,
            self.pub_leaves,
            self.ev_gps,
            self.ev_gps_fix,
            self.ev_gps_fix_t,
            self.ev_gps_fix_lat,
            self.ev_gps_fix_lon,
            self.ev_gps_fix_alt,
            self.ev_gps_velocity,
            self.ev_gps_velocity_speed,
            self.ev_gps_velocity_heading,
            self.ev_wake,
            self.fix_leaves,
            self.fix_t,
            self.fix_lat,
            self.fix_lon,
            self.fix_alt,
            self.gps_max_items,
            self.pub_max_bytes,
            self.event_max_bytes,
            self.state_free_strings,
        ]
    }

    /// `layout.bin`'s fixed part's bytes (all of a version 1 layout).
    pub fn to_bytes(&self) -> [u8; BYTES] {
        let mut b = [0; BYTES];
        for (i, v) in self.fields().iter().enumerate() {
            b[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        b[HASH_AT..].copy_from_slice(&self.interface_hash);
        b
    }
}
