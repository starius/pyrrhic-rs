use std::{
    cell::UnsafeCell,
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    os::raw::c_char,
    path::PathBuf,
    sync::OnceLock,
};

use crate::{
    storage::TableBytes,
    table_decoder::decode_pair,
    table_encoder::{encode_squares, fill_squares, leading_pawn},
    table_moves::{generate_captures, generate_moves, MoveError, MoveList},
    table_parser::{parse_table, Description, ParsedTable},
};

#[derive(Copy, Clone, Eq, PartialEq)]
#[repr(i32)]
enum TableType {
    Wdl = 0,
    Dtz = 1,
}
const TB_MIN_FILE_SIZE: u64 = 80;

pub(crate) const PYRRHIC_PRIME_BPAWN: u64 = 11695583624105689831;
pub(crate) const PYRRHIC_BPAWN: u32 = 9;
pub(crate) const PYRRHIC_PRIME_BKNIGHT: u64 = 13469005675588064321;
pub(crate) const PYRRHIC_BKNIGHT: u32 = 10;
pub(crate) const PYRRHIC_PRIME_BBISHOP: u64 = 15394650811035483107;
pub(crate) const PYRRHIC_BBISHOP: u32 = 11;
pub(crate) const PYRRHIC_PRIME_BROOK: u64 = 18264461213049635989;
pub(crate) const PYRRHIC_BROOK: u32 = 12;
pub(crate) const PYRRHIC_PRIME_BQUEEN: u64 = 15484752644942473553;
pub(crate) const PYRRHIC_BQUEEN: u32 = 13;
pub(crate) const PYRRHIC_PRIME_WPAWN: u64 = 17008651141875982339;
pub(crate) const PYRRHIC_WPAWN: u32 = 1;
pub(crate) const PYRRHIC_PRIME_WKNIGHT: u64 = 15202887380319082783;
pub(crate) const PYRRHIC_WKNIGHT: u32 = 2;
pub(crate) const PYRRHIC_PRIME_WBISHOP: u64 = 12311744257139811149;
pub(crate) const PYRRHIC_WBISHOP: u32 = 3;
pub(crate) const PYRRHIC_PRIME_WROOK: u64 = 10979190538029446137;
pub(crate) const PYRRHIC_WROOK: u32 = 4;
pub(crate) const PYRRHIC_PRIME_WQUEEN: u64 = 11811845319353239651;
pub(crate) const PYRRHIC_WQUEEN: u32 = 5;

#[repr(C)]
pub(crate) struct BaseEntry {
    pub(crate) key: u64,
    loaded: [OnceLock<Result<LoadedTable, LoadError>>; 2],
    pub(crate) num: u8,
    pub(crate) symmetric: bool,
    pub(crate) hasPawns: bool,
    pub(crate) hasDtz: bool,
    pub(crate) c2rust_unnamed: C2RustUnnamed_0,
}

impl BaseEntry {
    fn initialized() -> Self {
        Self {
            key: 0,
            loaded: std::array::from_fn(|_| OnceLock::new()),
            num: 0,
            symmetric: false,
            hasPawns: false,
            hasDtz: false,
            c2rust_unnamed: C2RustUnnamed_0 { kk_enc: false },
        }
    }
}
#[derive(Copy, Clone)]
#[repr(C)]
pub(crate) union C2RustUnnamed_0 {
    pub(crate) kk_enc: bool,
    pub(crate) pawns: [u8; 2],
}

#[repr(C)]
pub(crate) struct PieceEntry {
    pub(crate) be: BaseEntry,
}

impl PieceEntry {
    fn initialized() -> Self {
        Self {
            be: BaseEntry::initialized(),
        }
    }
}
#[repr(C)]
pub(crate) struct PawnEntry {
    pub(crate) be: BaseEntry,
}

impl PawnEntry {
    fn initialized() -> Self {
        Self {
            be: BaseEntry::initialized(),
        }
    }
}

#[derive(Copy, Clone)]
enum LoadError {
    MissingOrInvalid,
}

enum LoadedStorage {
    Piece(Box<PieceEntry>),
    Pawn(Box<PawnEntry>),
}

struct LoadedTable {
    storage: LoadedStorage,
    backing: Option<TableBytes>,
    parsed: Option<ParsedTable>,
    description: Description,
}

impl LoadedTable {
    fn new(original: *const BaseEntry, description: Description) -> Self {
        let storage = if unsafe { (*original).hasPawns } {
            LoadedStorage::Pawn(Box::new(PawnEntry::initialized()))
        } else {
            LoadedStorage::Piece(Box::new(PieceEntry::initialized()))
        };
        let mut table = Self {
            storage,
            backing: None,
            parsed: None,
            description,
        };
        let destination = table.entry_ptr_mut();
        unsafe {
            (*destination).key = (*original).key;
            (*destination).num = (*original).num;
            (*destination).symmetric = (*original).symmetric;
            (*destination).hasPawns = (*original).hasPawns;
            (*destination).hasDtz = (*original).hasDtz;
            if (*original).hasPawns {
                (*destination).c2rust_unnamed.pawns = (*original).c2rust_unnamed.pawns;
            } else {
                (*destination).c2rust_unnamed.kk_enc = (*original).c2rust_unnamed.kk_enc;
            }
        }
        table
    }

    fn entry_ptr(&self) -> *mut BaseEntry {
        match &self.storage {
            LoadedStorage::Piece(entry) => (&raw const entry.be).cast_mut(),
            LoadedStorage::Pawn(entry) => (&raw const entry.be).cast_mut(),
        }
    }

    fn entry_ptr_mut(&mut self) -> *mut BaseEntry {
        match &mut self.storage {
            LoadedStorage::Piece(entry) => &raw mut entry.be,
            LoadedStorage::Pawn(entry) => &raw mut entry.be,
        }
    }

    fn install(&mut self, backing: TableBytes, parsed: ParsedTable) {
        self.parsed = Some(parsed);
        self.backing = Some(backing);
    }
}

// Each table is fully constructed before OnceLock publishes it. Its raw
// pointers target only the immutable mapping and owned metadata above. This
// temporary bridge is removed when the decoder and encoder become safe.
unsafe impl Send for LoadedTable {}
unsafe impl Sync for LoadedTable {}

#[derive(Copy, Clone)]
enum EntryIndex {
    Piece(usize),
    Pawn(usize),
}

#[derive(Copy, Clone)]
struct TbHashEntry {
    key: u64,
    entry: Option<EntryIndex>,
}
pub(crate) const DTZ: u32 = 1;
#[derive(Copy, Clone)]
#[repr(C)]
pub(crate) struct stat {
    pub(crate) st_dev: u64,
    pub(crate) st_ino: u64,
    pub(crate) st_nlink: u64,
    pub(crate) st_mode: u32,
    pub(crate) st_uid: u32,
    pub(crate) st_gid: u32,
    pub(crate) __pad0: i32,
    pub(crate) st_rdev: u64,
    pub(crate) st_size: i64,
    pub(crate) st_blksize: i64,
    pub(crate) st_blocks: i64,
    pub(crate) st_atime: i64,
    pub(crate) st_atimensec: u64,
    pub(crate) st_mtime: i64,
    pub(crate) st_mtimensec: u64,
    pub(crate) st_ctime: i64,
    pub(crate) st_ctimensec: u64,
    pub(crate) __glibc_reserved: [i64; 3],
}
pub(crate) const PYRRHIC_PAWN: u32 = 1;
pub(crate) const PYRRHIC_KING: u32 = 6;
pub(crate) const WDL: u32 = 0;
pub(crate) const PYRRHIC_QUEEN: u32 = 5;

#[derive(Copy, Clone)]
#[repr(C)]
pub(crate) struct PyrrhicPosition {
    pub(crate) white: u64,
    pub(crate) black: u64,
    pub(crate) kings: u64,
    pub(crate) queens: u64,
    pub(crate) rooks: u64,
    pub(crate) bishops: u64,
    pub(crate) knights: u64,
    pub(crate) pawns: u64,
    pub(crate) rule50: u8,
    pub(crate) ep: u8,
    pub(crate) turn: bool,
}
pub(crate) const PIECE_ENC: u32 = 0;
pub(crate) const FILE_ENC: u32 = 1;
pub(crate) const PYRRHIC_WHITE: u32 = 1;
pub(crate) const PYRRHIC_ROOK: u32 = 4;
pub(crate) const PYRRHIC_BISHOP: u32 = 3;
pub(crate) const PYRRHIC_KNIGHT: u32 = 2;
pub(crate) const PYRRHIC_BLACK: u32 = 0;
pub(crate) const PYRRHIC_PRIME_NONE: u64 = 0;
pub(crate) const PYRRHIC_PRIME_BKING: u64 = 0;
pub(crate) const PYRRHIC_PRIME_WKING: u64 = 0;
pub(crate) type PyrrhicMove = u16;
pub(crate) const PYRRHIC_PROMOSQS: u64 = 18374686479671623935;
pub(crate) const PYRRHIC_PROMOTES_BISHOP: u32 = 3;
pub(crate) const PYRRHIC_PROMOTES_ROOK: u32 = 2;
pub(crate) const PYRRHIC_PROMOTES_KNIGHT: u32 = 4;
pub(crate) const PYRRHIC_PROMOTES_QUEEN: u32 = 1;
pub(crate) const PYRRHIC_PROMOTES_NONE: u32 = 0;
pub(crate) const PYRRHIC_BKING: u32 = 14;
pub(crate) const PYRRHIC_WKING: u32 = 6;
pub(crate) fn poplsb(x: &mut u64) -> u64 {
    let lsb = x.trailing_zeros();
    *x &= x.wrapping_sub(1);
    lsb as u64
}

use crate::engine_adapter::{Color, EngineAdapter};

pub(crate) fn popcount(x: u64) -> u64 {
    x.count_ones() as u64
}

pub(crate) fn getlsb(x: u64) -> u64 {
    x.trailing_zeros() as u64
}

// Windows drive letters contain ':', so its tablebase path list uses ';'.
const PATH_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// One immutable discovery set with lazily initialized, synchronized mappings.
/// The entry arrays are allocated once and their addresses remain stable until
/// the final handle is dropped.
pub(crate) struct GenerationData {
    paths: Vec<PathBuf>,
    discovered: Vec<(String, bool)>,
    pub(crate) max_cardinality: i32,
    pub(crate) largest: i32,
    pub(crate) num_wdl: i32,
    pub(crate) num_dtz: i32,
    num_piece: i32,
    num_pawn: i32,
    piece_entry: Box<[PieceEntry]>,
    pawn_entry: Box<[PawnEntry]>,
    hash: [TbHashEntry; 4096],
}

impl Default for GenerationData {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            discovered: Vec::new(),
            max_cardinality: 0,
            largest: 0,
            num_wdl: 0,
            num_dtz: 0,
            num_piece: 0,
            num_pawn: 0,
            piece_entry: Vec::new().into_boxed_slice(),
            pawn_entry: Vec::new().into_boxed_slice(),
            hash: [TbHashEntry {
                key: 0,
                entry: None,
            }; 4096],
        }
    }
}

pub(crate) struct Generation(UnsafeCell<GenerationData>);

impl Generation {
    pub(crate) fn new() -> Self {
        Self(UnsafeCell::new(GenerationData::default()))
    }

    fn state_ptr(&self) -> *mut GenerationData {
        self.0.get()
    }

    pub(crate) fn max_pieces(&self) -> u32 {
        // Discovery is complete before the owner is shared; this field is
        // never written after publication.
        unsafe { (*self.0.get()).largest as u32 }
    }

    pub(crate) fn materials(&self) -> Vec<(String, bool)> {
        // Discovery is complete before this is called. Clone only at load
        // time; probing never needs the material-name list.
        unsafe { (*self.0.get()).discovered.clone() }
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (i32, i32) {
        unsafe { ((*self.0.get()).num_wdl, (*self.0.get()).num_dtz) }
    }
}

// Discovery finishes before a Generation is shared. The hash, paths, and
// counts are then read-only. Lazy mappings are published through entry cells.
// Arc destruction occurs after every active probe and clone has released it.
unsafe impl Send for Generation {}
unsafe impl Sync for Generation {}

unsafe fn open_tb(
    owner: &Generation,
    mut str: *const c_char,
    mut suffix: *const c_char,
) -> Result<File, std::io::Error> {
    let state = owner.state_ptr();
    for path in &(*state).paths {
        let str = CStr::from_ptr(str);
        let suffix = CStr::from_ptr(suffix);
        let file = path.join(format!(
            "{}{}",
            str.to_str().unwrap(),
            suffix.to_str().unwrap()
        ));
        let file_handle = OpenOptions::new().read(true).open(file);
        if file_handle.is_ok() {
            return file_handle;
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "No tablebase files found",
    ))
}
fn close_tb(_file_handle: File) {}

const TB_SUFFIX: [*const c_char; 2] = [
    b".rtbw\0" as *const u8 as *const c_char,
    b".rtbz\0" as *const u8 as *const c_char,
];
const TB_MAGIC: [u32; 2] = [0x5d23e871, 0xa50c66d7];

pub(crate) fn pyrrhic_move_from(move_0: PyrrhicMove) -> u32 {
    (move_0 as i32 >> 6 & 0x3f) as u32
}

pub(crate) fn pyrrhic_move_to(move_0: PyrrhicMove) -> u32 {
    (move_0 & 0x3f) as u32
}

pub(crate) fn pyrrhic_move_promotes(move_0: PyrrhicMove) -> u32 {
    (move_0 as i32 >> 12 & 0x7) as u32
}

pub(crate) fn pyrrhic_colour_of_piece(piece: u8) -> i32 {
    (piece as i32 >> 3 == 0) as i32
}

pub(crate) fn pyrrhic_type_of_piece(piece: u8) -> i32 {
    piece as i32 & 0x7
}

pub(crate) fn pyrrhic_test_bit(bb: u64, sq: i32) -> bool {
    bb >> sq & 0x1 != 0
}

pub(crate) fn pyrrhic_enable_bit(b: &mut u64, sq: i32) {
    *b |= 1 << sq;
}

pub(crate) fn pyrrhic_disable_bit(b: &mut u64, sq: i32) {
    *b &= !(1 << sq);
}

pub(crate) fn pyrrhic_promo_square(sq: i32) -> bool {
    PYRRHIC_PROMOSQS >> sq & 0x1 != 0
}

pub(crate) fn pyrrhic_pawn_start_square(colour: i32, sq: i32) -> bool {
    sq >> 3 == (if colour != 0 { 1 } else { 6 })
}

pub(crate) static pyrrhic_piece_to_char: [c_char; 16] =
    unsafe { *::core::mem::transmute::<&[u8; 16], &[c_char; 16]>(b" PNBRQK  pnbrqk\0") };

pub(crate) unsafe fn pyrrhic_pieces_by_type(
    pos: *const PyrrhicPosition,
    colour: i32,
    piece: i32,
) -> u64 {
    assert!(PYRRHIC_PAWN as i32 <= piece && piece <= PYRRHIC_KING as i32);
    assert!(colour == PYRRHIC_WHITE as i32 || colour == PYRRHIC_BLACK as i32);
    let mut side = if colour == PYRRHIC_WHITE as i32 {
        (*pos).white
    } else {
        (*pos).black
    };
    match piece {
        1 => (*pos).pawns & side,
        2 => (*pos).knights & side,
        3 => (*pos).bishops & side,
        4 => (*pos).rooks & side,
        5 => (*pos).queens & side,
        6 => (*pos).kings & side,
        _ => unreachable!(),
    }
}

pub(crate) fn pyrrhic_char_to_piece_type(c: c_char) -> i32 {
    let mut i: i32 = PYRRHIC_PAWN as i32;
    while i <= PYRRHIC_KING as i32 {
        if c as i32 == pyrrhic_piece_to_char[i as usize] as i32 {
            return i;
        }
        i += 1;
    }
    0
}

pub(crate) unsafe fn pyrrhic_calc_key(pos: *const PyrrhicPosition, mirror: i32) -> u64 {
    let mut white: u64 = if mirror != 0 {
        (*pos).black
    } else {
        (*pos).white
    };
    let mut black: u64 = if mirror != 0 {
        (*pos).white
    } else {
        (*pos).black
    };
    (popcount(white & (*pos).queens))
        .wrapping_mul(PYRRHIC_PRIME_WQUEEN)
        .wrapping_add((popcount(white & (*pos).rooks)).wrapping_mul(PYRRHIC_PRIME_WROOK))
        .wrapping_add((popcount(white & (*pos).bishops)).wrapping_mul(PYRRHIC_PRIME_WBISHOP))
        .wrapping_add((popcount(white & (*pos).knights)).wrapping_mul(PYRRHIC_PRIME_WKNIGHT))
        .wrapping_add((popcount(white & (*pos).pawns)).wrapping_mul(PYRRHIC_PRIME_WPAWN))
        .wrapping_add((popcount(black & (*pos).queens)).wrapping_mul(PYRRHIC_PRIME_BQUEEN))
        .wrapping_add((popcount(black & (*pos).rooks)).wrapping_mul(PYRRHIC_PRIME_BROOK))
        .wrapping_add((popcount(black & (*pos).bishops)).wrapping_mul(PYRRHIC_PRIME_BBISHOP))
        .wrapping_add((popcount(black & (*pos).knights)).wrapping_mul(PYRRHIC_PRIME_BKNIGHT))
        .wrapping_add((popcount(black & (*pos).pawns)).wrapping_mul(PYRRHIC_PRIME_BPAWN))
}

pub(crate) unsafe fn pyrrhic_calc_key_from_pcs(pieces: *mut i32, mirror: i32) -> u64 {
    (*pieces.offset((PYRRHIC_WQUEEN as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize) as u64)
        .wrapping_mul(PYRRHIC_PRIME_WQUEEN)
        .wrapping_add(
            (*pieces.offset((PYRRHIC_WROOK as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_WROOK),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_WBISHOP as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_WBISHOP),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_WKNIGHT as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_WKNIGHT),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_WPAWN as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_WPAWN),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_BQUEEN as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_BQUEEN),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_BROOK as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_BROOK),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_BBISHOP as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_BBISHOP),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_BKNIGHT as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_BKNIGHT),
        )
        .wrapping_add(
            (*pieces.offset((PYRRHIC_BPAWN as i32 ^ (if mirror != 0 { 8 } else { 0 })) as isize)
                as u64)
                .wrapping_mul(PYRRHIC_PRIME_BPAWN),
        )
}

pub(crate) unsafe fn pyrrhic_calc_key_from_pieces(pieces: *mut u8, length: i32) -> u64 {
    const PYRRHIC_PRIMES: [u64; 16] = [
        PYRRHIC_PRIME_NONE,
        PYRRHIC_PRIME_WPAWN,
        PYRRHIC_PRIME_WKNIGHT,
        PYRRHIC_PRIME_WBISHOP,
        PYRRHIC_PRIME_WROOK,
        PYRRHIC_PRIME_WQUEEN,
        PYRRHIC_PRIME_WKING,
        PYRRHIC_PRIME_NONE,
        PYRRHIC_PRIME_NONE,
        PYRRHIC_PRIME_BPAWN,
        PYRRHIC_PRIME_BKNIGHT,
        PYRRHIC_PRIME_BBISHOP,
        PYRRHIC_PRIME_BROOK,
        PYRRHIC_PRIME_BQUEEN,
        PYRRHIC_PRIME_BKING,
        PYRRHIC_PRIME_NONE,
    ];
    let mut key = 0u64;
    let mut i = 0;
    while i < length {
        key = key.wrapping_add(PYRRHIC_PRIMES[*pieces.offset(i as isize) as usize]);
        i += 1;
    }
    key
}

pub(crate) fn pyrrhic_do_bb_move(bb: u64, from: u32, to: u32) -> u64 {
    ((bb >> from & 0x1) << to) | bb & (!(1 << from) & !(1 << to))
}

pub(crate) fn pyrrhic_make_move(promote: u32, from: u32, to: u32) -> PyrrhicMove {
    ((promote & 0x7) << 12 | (from & 0x3f) << 6 | to & 0x3f) as PyrrhicMove
}

unsafe fn generate_legal<E: EngineAdapter>(
    position: &PyrrhicPosition,
) -> Result<MoveList<256>, MoveError> {
    let pseudo = generate_moves::<E>(position)?;
    let mut legal = MoveList::new();
    for &candidate in pseudo.as_slice() {
        if pyrrhic_legal_move::<E>(position, candidate) {
            legal.push(candidate)?;
        }
    }
    Ok(legal)
}

pub(crate) unsafe fn pyrrhic_is_pawn_move(
    mut pos: *const PyrrhicPosition,
    mut move_0: PyrrhicMove,
) -> bool {
    let mut us: u64 = if (*pos).turn as i32 != 0 {
        (*pos).white
    } else {
        (*pos).black
    };
    pyrrhic_test_bit(us & (*pos).pawns, pyrrhic_move_from(move_0) as i32)
}

pub(crate) unsafe fn pyrrhic_is_en_passant(
    mut pos: *const PyrrhicPosition,
    mut move_0: PyrrhicMove,
) -> bool {
    pyrrhic_is_pawn_move(pos, move_0) as i32 != 0
        && pyrrhic_move_to(move_0) == (*pos).ep as u32
        && (*pos).ep as i32 != 0
}

pub(crate) unsafe fn pyrrhic_is_capture(
    mut pos: *const PyrrhicPosition,
    mut move_0: PyrrhicMove,
) -> bool {
    let mut them: u64 = if (*pos).turn as i32 != 0 {
        (*pos).black
    } else {
        (*pos).white
    };
    pyrrhic_test_bit(them, pyrrhic_move_to(move_0) as i32) as i32 != 0
        || pyrrhic_is_en_passant(pos, move_0) as i32 != 0
}

pub(crate) unsafe fn pyrrhic_is_legal<E: EngineAdapter>(mut pos: *const PyrrhicPosition) -> bool {
    let mut us: u64 = if (*pos).turn as i32 != 0 {
        (*pos).black
    } else {
        (*pos).white
    };
    let mut them: u64 = if (*pos).turn as i32 != 0 {
        (*pos).white
    } else {
        (*pos).black
    };
    let mut sq: u32 = getlsb((*pos).kings & us) as u32;
    E::king_attacks(sq as u64) & (*pos).kings & them == 0
        && E::rook_attacks(sq as u64, us | them) & ((*pos).rooks | (*pos).queens) & them == 0
        && E::bishop_attacks(sq as u64, us | them) & ((*pos).bishops | (*pos).queens) & them == 0
        && E::knight_attacks(sq as u64) & (*pos).knights & them == 0
        && E::pawn_attacks(
            if (*pos).turn {
                Color::Black
            } else {
                Color::White
            },
            sq as u64,
        ) & (*pos).pawns
            & them
            == 0
}

pub(crate) unsafe fn pyrrhic_is_check<E: EngineAdapter>(mut pos: *const PyrrhicPosition) -> bool {
    let mut us: u64 = if (*pos).turn as i32 != 0 {
        (*pos).white
    } else {
        (*pos).black
    };
    let mut them: u64 = if (*pos).turn as i32 != 0 {
        (*pos).black
    } else {
        (*pos).white
    };
    let mut sq: u32 = getlsb((*pos).kings & us) as u32;
    E::rook_attacks(sq as u64, us | them) & (((*pos).rooks | (*pos).queens) & them) != 0
        || E::bishop_attacks(sq as u64, us | them) & (((*pos).bishops | (*pos).queens) & them) != 0
        || E::knight_attacks(sq as u64) & ((*pos).knights & them) != 0
        || E::pawn_attacks(
            if (*pos).turn {
                Color::White
            } else {
                Color::Black
            },
            sq as u64,
        ) & ((*pos).pawns & them)
            != 0
}

pub(crate) unsafe fn pyrrhic_is_mate<E: EngineAdapter>(
    pos: *const PyrrhicPosition,
) -> Result<bool, MoveError> {
    if !pyrrhic_is_check::<E>(pos) {
        return Ok(false);
    }
    let mut pos1: PyrrhicPosition = PyrrhicPosition {
        white: 0,
        black: 0,
        kings: 0,
        queens: 0,
        rooks: 0,
        bishops: 0,
        knights: 0,
        pawns: 0,
        rule50: 0,
        ep: 0,
        turn: false,
    };
    let moves = generate_moves::<E>(&*pos)?;
    for &candidate in moves.as_slice() {
        if pyrrhic_do_move::<E>(&mut pos1, pos, candidate) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) unsafe fn pyrrhic_do_move<E: EngineAdapter>(
    mut pos: *mut PyrrhicPosition,
    mut pos0: *const PyrrhicPosition,
    mut move_0: PyrrhicMove,
) -> bool {
    let mut from: u32 = pyrrhic_move_from(move_0);
    let mut to: u32 = pyrrhic_move_to(move_0);
    let mut promotes: u32 = pyrrhic_move_promotes(move_0);
    (*pos).turn = !(*pos0).turn;
    (*pos).white = pyrrhic_do_bb_move((*pos0).white, from, to);
    (*pos).black = pyrrhic_do_bb_move((*pos0).black, from, to);
    (*pos).kings = pyrrhic_do_bb_move((*pos0).kings, from, to);
    (*pos).queens = pyrrhic_do_bb_move((*pos0).queens, from, to);
    (*pos).rooks = pyrrhic_do_bb_move((*pos0).rooks, from, to);
    (*pos).bishops = pyrrhic_do_bb_move((*pos0).bishops, from, to);
    (*pos).knights = pyrrhic_do_bb_move((*pos0).knights, from, to);
    (*pos).pawns = pyrrhic_do_bb_move((*pos0).pawns, from, to);
    (*pos).ep = 0;
    if promotes != PYRRHIC_PROMOTES_NONE as i32 as u32 {
        pyrrhic_disable_bit(&mut (*pos).pawns, to as i32);
        match promotes {
            1 => {
                pyrrhic_enable_bit(&mut (*pos).queens, to as i32);
            }
            2 => {
                pyrrhic_enable_bit(&mut (*pos).rooks, to as i32);
            }
            3 => {
                pyrrhic_enable_bit(&mut (*pos).bishops, to as i32);
            }
            4 => {
                pyrrhic_enable_bit(&mut (*pos).knights, to as i32);
            }
            _ => {}
        }
        (*pos).rule50 = 0;
    } else if pyrrhic_test_bit((*pos0).pawns, from as i32) {
        (*pos).rule50 = 0;
        let opposing_pawns = (*pos0).pawns
            & if (*pos0).turn {
                (*pos0).black
            } else {
                (*pos0).white
            };
        (*pos).ep = ep_after_double_push::<E>(from, to, (*pos0).turn, opposing_pawns);
        if to == (*pos0).ep as u32 {
            pyrrhic_disable_bit(
                &mut (*pos).white,
                (if (*pos0).turn as i32 != 0 {
                    to.wrapping_sub(8)
                } else {
                    to.wrapping_add(8)
                }) as i32,
            );
            pyrrhic_disable_bit(
                &mut (*pos).black,
                (if (*pos0).turn as i32 != 0 {
                    to.wrapping_sub(8)
                } else {
                    to.wrapping_add(8)
                }) as i32,
            );
            pyrrhic_disable_bit(
                &mut (*pos).pawns,
                (if (*pos0).turn as i32 != 0 {
                    to.wrapping_sub(8)
                } else {
                    to.wrapping_add(8)
                }) as i32,
            );
        }
    } else if pyrrhic_test_bit((*pos0).white | (*pos0).black, to as i32) {
        (*pos).rule50 = 0;
    } else {
        (*pos).rule50 = ((*pos0).rule50 as i32 + 1) as u8;
    }
    pyrrhic_is_legal::<E>(pos)
}

fn ep_after_double_push<E: EngineAdapter>(
    from: u32,
    to: u32,
    white_moved: bool,
    opposing_pawns: u64,
) -> u8 {
    if from ^ to != 16 {
        return 0;
    }
    let ep = if white_moved {
        from.checked_add(8)
    } else {
        from.checked_sub(8)
    };
    let Some(ep) = ep.filter(|&square| square < 64) else {
        return 0;
    };
    // Look backward from the target square with the mover's attack
    // direction to find opposing pawns that can capture en passant.
    let mover = if white_moved {
        Color::White
    } else {
        Color::Black
    };
    if E::pawn_attacks(mover, ep as u64) & opposing_pawns != 0 {
        ep as u8
    } else {
        0
    }
}

#[cfg(test)]
mod double_push_tests {
    use super::*;

    #[derive(Clone)]
    struct AttackOnly;

    impl EngineAdapter for AttackOnly {
        fn pawn_attacks(color: Color, square: u64) -> u64 {
            let square = cozy_chess::Square::index(square as usize);
            let color = if color == Color::White {
                cozy_chess::Color::White
            } else {
                cozy_chess::Color::Black
            };
            cozy_chess::get_pawn_attacks(square, color).0
        }

        fn knight_attacks(_: u64) -> u64 {
            unreachable!()
        }
        fn bishop_attacks(_: u64, _: u64) -> u64 {
            unreachable!()
        }
        fn rook_attacks(_: u64, _: u64) -> u64 {
            unreachable!()
        }
        fn queen_attacks(_: u64, _: u64) -> u64 {
            unreachable!()
        }
        fn king_attacks(_: u64) -> u64 {
            unreachable!()
        }
    }

    #[test]
    fn double_push_retains_ep_only_for_an_opposing_capturer() {
        // The reverse lookup uses the mover's attack direction from the EP
        // square. These are move-state transitions, not root-move fixtures.
        assert_eq!(ep_after_double_push::<AttackOnly>(8, 24, true, 1 << 25), 16);
        assert_eq!(
            ep_after_double_push::<AttackOnly>(15, 31, true, 1 << 30),
            23
        );
        assert_eq!(
            ep_after_double_push::<AttackOnly>(53, 37, false, 1 << 36),
            45
        );
        assert_eq!(
            ep_after_double_push::<AttackOnly>(48, 32, false, 1 << 33),
            40
        );
        assert_eq!(ep_after_double_push::<AttackOnly>(8, 24, true, 0), 0);
        assert_eq!(ep_after_double_push::<AttackOnly>(8, 24, true, 1 << 9), 0);
        assert_eq!(ep_after_double_push::<AttackOnly>(8, 16, true, 1 << 25), 0);
    }
}

pub(crate) unsafe fn pyrrhic_legal_move<E: EngineAdapter>(
    mut pos: *const PyrrhicPosition,
    mut move_0: PyrrhicMove,
) -> bool {
    let mut pos1: PyrrhicPosition = PyrrhicPosition {
        white: 0,
        black: 0,
        kings: 0,
        queens: 0,
        rooks: 0,
        bishops: 0,
        knights: 0,
        pawns: 0,
        rule50: 0,
        ep: 0,
        turn: false,
    };
    pyrrhic_do_move::<E>(&mut pos1, pos, move_0)
}
unsafe fn dtz_to_wdl(mut cnt50: i32, mut dtz: i32) -> u32 {
    let mut wdl = 0;
    if dtz > 0 {
        wdl = if dtz + cnt50 <= 100 as i32 { 2 } else { 1 };
    } else if dtz < 0 {
        wdl = if -dtz + cnt50 <= 100 as i32 {
            -2 as i32
        } else {
            -1 as i32
        };
    }
    (wdl + 2) as u32
}
pub(crate) unsafe fn tb_probe_wdl<E: EngineAdapter>(
    owner: &Generation,
    mut white: u64,
    mut black: u64,
    mut kings: u64,
    mut queens: u64,
    mut rooks: u64,
    mut bishops: u64,
    mut knights: u64,
    mut pawns: u64,
    mut ep: u32,
    mut turn: bool,
) -> u32 {
    let mut pos: PyrrhicPosition = {
        PyrrhicPosition {
            white,
            black,
            kings,
            queens,
            rooks,
            bishops,
            knights,
            pawns,
            rule50: 0,
            ep: ep as u8,
            turn,
        }
    };
    let mut success: i32 = 0;
    let mut v: i32 = probe_wdl::<E>(owner, &mut pos, &mut success);
    if success == 0 {
        return 0xffffffff;
    }
    (v + 2) as u32
}

/// Signed DTZ from the side to move, without applying a halfmove clock.
/// The caller can combine this with its own rule-50 policy.
pub(crate) unsafe fn tb_probe_dtz<E: EngineAdapter>(
    owner: &Generation,
    white: u64,
    black: u64,
    kings: u64,
    queens: u64,
    rooks: u64,
    bishops: u64,
    knights: u64,
    pawns: u64,
    ep: u32,
    turn: bool,
) -> Option<i32> {
    let mut pos = PyrrhicPosition {
        white,
        black,
        kings,
        queens,
        rooks,
        bishops,
        knights,
        pawns,
        rule50: 0,
        ep: ep as u8,
        turn,
    };
    let mut success = 0;
    let dtz = probe_dtz::<E>(owner, &mut pos, &mut success);
    (success != 0).then_some(dtz)
}
pub(crate) unsafe fn tb_probe_root<E: EngineAdapter>(
    owner: &Generation,
    mut white: u64,
    mut black: u64,
    mut kings: u64,
    mut queens: u64,
    mut rooks: u64,
    mut bishops: u64,
    mut knights: u64,
    mut pawns: u64,
    mut rule50: u32,
    mut ep: u32,
    mut turn: bool,
    mut results: *mut u32,
) -> u32 {
    let mut pos: PyrrhicPosition = {
        PyrrhicPosition {
            white,
            black,
            kings,
            queens,
            rooks,
            bishops,
            knights,
            pawns,
            rule50: rule50 as u8,
            ep: ep as u8,
            turn,
        }
    };
    let mut dtz: i32 = 0;
    let mut move_0: PyrrhicMove = probe_root::<E>(owner, &mut pos, &mut dtz, results);
    if move_0 as i32 == 0 {
        return 0xffffffff;
    }
    if move_0 as i32 == 0xfffe {
        return 4;
    }
    if move_0 as i32 == 0xffff {
        return 2 & 0xf_u32;
    }
    let mut res: u32 = 0;
    res = res & !0xf | dtz_to_wdl(rule50 as i32, dtz) & 0xf;
    res = res & !0xfff00000 | ((if dtz < 0 { -dtz } else { dtz }) << 20) as u32 & 0xfff00000;
    res = res & !0xfc00 | pyrrhic_move_from(move_0) << 10 & 0xfc00;
    res = res & !0x3f0 | pyrrhic_move_to(move_0) << 4 & 0x3f0;
    res = res & !0x70000 | pyrrhic_move_promotes(move_0) << 16 & 0x70000;
    res = res & !0x80000 | ((pyrrhic_is_en_passant(&pos, move_0) as i32) << 19 & 0x80000) as u32;
    res
}
unsafe fn prt_str(mut pos: *const PyrrhicPosition, mut str: *mut c_char, mut flip: i32) {
    let mut color: i32 = if flip != 0 {
        PYRRHIC_BLACK as i32
    } else {
        PYRRHIC_WHITE as i32
    };
    let mut pt: i32 = PYRRHIC_KING as i32;
    while pt >= PYRRHIC_PAWN as i32 {
        let mut i: i32 = popcount(pyrrhic_pieces_by_type(pos, color, pt)) as i32;
        while i > 0 {
            let fresh6 = str;
            str = str.offset(1);
            *fresh6 = pyrrhic_piece_to_char[pt as usize];
            i -= 1;
        }
        pt -= 1;
    }
    let fresh7 = str;
    str = str.offset(1);
    *fresh7 = 'v' as i32 as c_char;
    let mut pt_0: i32 = PYRRHIC_KING as i32;
    while pt_0 >= PYRRHIC_PAWN as i32 {
        let mut i_0: i32 = popcount(pyrrhic_pieces_by_type(pos, color ^ 1, pt_0)) as i32;
        while i_0 > 0 {
            let fresh8 = str;
            str = str.offset(1);
            *fresh8 = pyrrhic_piece_to_char[pt_0 as usize];
            i_0 -= 1;
        }
        pt_0 -= 1;
    }
    let fresh9 = str;
    str = str.offset(1);
    *fresh9 = 0;
}
unsafe fn test_tb(owner: &Generation, mut str: *const c_char, mut suffix: *const c_char) -> i32 {
    let mut file = open_tb(owner, str, suffix);
    if let Ok(file) = file {
        let Ok(metadata) = file.metadata() else {
            return -1;
        };
        let size = metadata.len();
        close_tb(file);
        if size < TB_MIN_FILE_SIZE || size & 63 != 16 {
            let file_path = format!(
                "{}.{}",
                CStr::from_ptr(str).to_str().unwrap(),
                CStr::from_ptr(suffix).to_str().unwrap()
            );
            eprintln!("Incomplete tablebase file {file_path}");
            println!("info string Incomplete tablebase file {file_path}");
            return -1;
        }
        1
    } else {
        -1
    }
}
unsafe fn add_to_hash(owner: &Generation, entry: EntryIndex, key: u64) {
    let mut idx: i32 = 0;
    idx = (key >> (64 - 12)) as i32;
    while (*owner.state_ptr()).hash[idx as usize].entry.is_some() {
        idx = (idx + 1) & ((1 << 12) - 1);
    }
    (*owner.state_ptr()).hash[idx as usize].key = key;
    (*owner.state_ptr()).hash[idx as usize].entry = Some(entry);
}
unsafe fn init_tb(owner: &Generation, mut str: *const c_char) {
    if test_tb(owner, str, TB_SUFFIX[WDL as i32 as usize]) != 1 {
        return;
    }
    let mut pcs: [i32; 16] = [0; 16];
    let mut i: i32 = 0;
    while i < 16 {
        pcs[i as usize] = 0;
        i += 1;
    }
    let mut color: i32 = 0;
    let mut s: *const c_char = str;
    while *s != 0 {
        if *s as i32 == 'v' as i32 {
            color = 8;
        } else {
            let mut piece_type: i32 = pyrrhic_char_to_piece_type(*s);
            if piece_type != 0 {
                assert!(piece_type | color < 16);
                pcs[(piece_type | color) as usize] += 1;
            }
        }
        s = s.offset(1);
    }
    let mut key: u64 = pyrrhic_calc_key_from_pcs(pcs.as_mut_ptr(), 0);
    let mut key2: u64 = pyrrhic_calc_key_from_pcs(pcs.as_mut_ptr(), 1);
    let mut hasPawns: bool =
        pcs[PYRRHIC_WPAWN as i32 as usize] != 0 || pcs[PYRRHIC_BPAWN as i32 as usize] != 0;
    let entry = if hasPawns as i32 != 0 {
        let fresh10 = (*owner.state_ptr()).num_pawn;
        (*owner.state_ptr()).num_pawn += 1;
        EntryIndex::Pawn(fresh10 as usize)
    } else {
        let fresh11 = (*owner.state_ptr()).num_piece;
        (*owner.state_ptr()).num_piece += 1;
        EntryIndex::Piece(fresh11 as usize)
    };
    let be: *mut BaseEntry = match entry {
        EntryIndex::Piece(index) => &mut (*owner.state_ptr()).piece_entry[index].be,
        EntryIndex::Pawn(index) => &mut (*owner.state_ptr()).pawn_entry[index].be,
    };
    (*be).hasPawns = hasPawns;
    (*be).key = key;
    (*be).symmetric = key == key2;
    (*be).num = 0;
    let mut i_0: i32 = 0;
    while i_0 < 16 {
        (*be).num = ((*be).num as i32 + pcs[i_0 as usize]) as u8;
        i_0 += 1;
    }
    (*owner.state_ptr()).num_wdl += 1;
    (*be).hasDtz = test_tb(owner, str, TB_SUFFIX[DTZ as i32 as usize]) == 1;
    (*owner.state_ptr()).num_dtz += (*be).hasDtz as i32;
    (*owner.state_ptr()).discovered.push((
        CStr::from_ptr(str).to_string_lossy().into_owned(),
        (*be).hasDtz,
    ));
    if (*be).num as i32 > (*owner.state_ptr()).max_cardinality {
        (*owner.state_ptr()).max_cardinality = (*be).num as i32;
    }
    if !(*be).hasPawns {
        let mut j: i32 = 0;
        let mut i_1: i32 = 0;
        while i_1 < 16 {
            if pcs[i_1 as usize] == 1 {
                j += 1;
            }
            i_1 += 1;
        }
        (*be).c2rust_unnamed.kk_enc = j == 2;
    } else {
        (*be).c2rust_unnamed.pawns[0] = pcs[PYRRHIC_WPAWN as i32 as usize] as u8;
        (*be).c2rust_unnamed.pawns[1] = pcs[PYRRHIC_BPAWN as i32 as usize] as u8;
        if pcs[PYRRHIC_BPAWN as i32 as usize] != 0
            && (pcs[PYRRHIC_WPAWN as i32 as usize] == 0
                || pcs[PYRRHIC_WPAWN as i32 as usize] > pcs[PYRRHIC_BPAWN as i32 as usize])
        {
            let mut tmp: i32 = (*be).c2rust_unnamed.pawns[0] as i32;
            (*be).c2rust_unnamed.pawns[0] = (*be).c2rust_unnamed.pawns[1];
            (*be).c2rust_unnamed.pawns[1] = tmp as u8;
        }
    }
    add_to_hash(owner, entry, key);
    if key != key2 {
        add_to_hash(owner, entry, key2);
    }
}

pub(crate) unsafe fn tb_init(owner: &Generation, path: &str) -> bool {
    if path.is_empty() || path == "<empty>" {
        return true;
    }
    let state = owner.state_ptr();
    for component in path
        .split(PATH_SEPARATOR)
        .filter(|component| !component.is_empty())
    {
        if component.contains('\0') {
            return false;
        }
        (*state).paths.push(PathBuf::from(component));
    }
    // The boxes keep entry addresses stable while the temporary raw hash
    // bridge is in use. Every entry and unused encoding tail is initialized.
    (*state).piece_entry = std::iter::repeat_with(PieceEntry::initialized)
        .take(650)
        .collect::<Vec<_>>()
        .into_boxed_slice();
    (*state).pawn_entry = std::iter::repeat_with(PawnEntry::initialized)
        .take(861)
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let mut i_4: i32 = 0;
    let mut j_0: i32 = 0;
    let mut k: i32 = 0;
    let mut l: i32 = 0;
    let mut m: i32 = 0;
    i_4 = 0;
    while i_4 < 5 {
        let str = CString::new(format!(
            "K{}vK",
            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8 as char
        ))
        .unwrap();
        init_tb(owner, str.as_ptr());
        i_4 += 1;
    }
    i_4 = 0;
    while i_4 < 5 {
        j_0 = i_4;
        while j_0 < 5 {
            let str = CString::new(format!(
                "K{}vK{}",
                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8 as char,
                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8 as char,
            ))
            .unwrap();
            init_tb(owner, str.as_ptr());
            j_0 += 1;
        }
        i_4 += 1;
    }
    i_4 = 0;
    while i_4 < 5 {
        j_0 = i_4;
        while j_0 < 5 {
            let str = CString::new(format!(
                "K{}{}vK",
                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8 as char,
                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8 as char,
            ))
            .unwrap();
            init_tb(owner, str.as_ptr());
            j_0 += 1;
        }
        i_4 += 1;
    }
    i_4 = 0;
    while i_4 < 5 {
        j_0 = i_4;
        while j_0 < 5 {
            k = 0;
            while k < 5 {
                let str = CString::new(format!(
                    "K{}{}vK{}",
                    pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8 as char,
                    pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8 as char,
                    pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8 as char,
                ))
                .unwrap();
                init_tb(owner, str.as_ptr());
                k += 1;
            }
            j_0 += 1;
        }
        i_4 += 1;
    }
    i_4 = 0;
    while i_4 < 5 {
        j_0 = i_4;
        while j_0 < 5 {
            k = j_0;
            while k < 5 {
                let str = CString::new(format!(
                    "K{}{}{}vK",
                    pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8 as char,
                    pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8 as char,
                    pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8 as char,
                ))
                .unwrap();
                init_tb(owner, str.as_ptr());
                k += 1;
            }
            j_0 += 1;
        }
        i_4 += 1;
    }
    if !((::core::mem::size_of::<u64>() as u64) < 8 || 7 < 6) {
        i_4 = 0;
        while i_4 < 5 {
            j_0 = i_4;
            while j_0 < 5 {
                k = i_4;
                while k < 5 {
                    l = if i_4 == k { j_0 } else { k };
                    while l < 5 {
                        let str = CString::new(format!(
                            "K{}{}vK{}{}",
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - l) as usize] as u8
                                as char,
                        ))
                        .unwrap();
                        init_tb(owner, str.as_ptr());
                        l += 1;
                    }
                    k += 1;
                }
                j_0 += 1;
            }
            i_4 += 1;
        }
        i_4 = 0;
        while i_4 < 5 {
            j_0 = i_4;
            while j_0 < 5 {
                k = j_0;
                while k < 5 {
                    l = 0;
                    while l < 5 {
                        let str = CString::new(format!(
                            "K{}{}{}vK{}",
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - l) as usize] as u8
                                as char,
                        ))
                        .unwrap();
                        init_tb(owner, str.as_ptr());
                        l += 1;
                    }
                    k += 1;
                }
                j_0 += 1;
            }
            i_4 += 1;
        }
        i_4 = 0;
        while i_4 < 5 {
            j_0 = i_4;
            while j_0 < 5 {
                k = j_0;
                while k < 5 {
                    l = k;
                    while l < 5 {
                        let str = CString::new(format!(
                            "K{}{}{}{}vK",
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8
                                as char,
                            pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - l) as usize] as u8
                                as char,
                        ))
                        .unwrap();
                        init_tb(owner, str.as_ptr());
                        l += 1;
                    }
                    k += 1;
                }
                j_0 += 1;
            }
            i_4 += 1;
        }
        i_4 = 0;
        while i_4 < 5 {
            j_0 = i_4;
            while j_0 < 5 {
                k = j_0;
                while k < 5 {
                    l = k;
                    while l < 5 {
                        m = l;
                        while m < 5 {
                            let str = CString::new(format!(
                                "K{}{}{}{}{}vK",
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - l) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - m) as usize] as u8
                                    as char,
                            ))
                            .unwrap();
                            init_tb(owner, str.as_ptr());
                            m += 1;
                        }
                        l += 1;
                    }
                    k += 1;
                }
                j_0 += 1;
            }
            i_4 += 1;
        }
        i_4 = 0;
        while i_4 < 5 {
            j_0 = i_4;
            while j_0 < 5 {
                k = j_0;
                while k < 5 {
                    l = k;
                    while l < 5 {
                        m = 0;
                        while m < 5 {
                            let str = CString::new(format!(
                                "K{}{}{}{}vK{}",
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - l) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - m) as usize] as u8
                                    as char,
                            ))
                            .unwrap();
                            init_tb(owner, str.as_ptr());
                            m += 1;
                        }
                        l += 1;
                    }
                    k += 1;
                }
                j_0 += 1;
            }
            i_4 += 1;
        }
        i_4 = 0;
        while i_4 < 5 {
            j_0 = i_4;
            while j_0 < 5 {
                k = j_0;
                while k < 5 {
                    l = 0;
                    while l < 5 {
                        m = l;
                        while m < 5 {
                            let str = CString::new(format!(
                                "K{}{}{}vK{}{}",
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - i_4) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - j_0) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - k) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - l) as usize] as u8
                                    as char,
                                pyrrhic_piece_to_char[(PYRRHIC_QUEEN as i32 - m) as usize] as u8
                                    as char,
                            ))
                            .unwrap();
                            init_tb(owner, str.as_ptr());
                            m += 1;
                        }
                        l += 1;
                    }
                    k += 1;
                }
                j_0 += 1;
            }
            i_4 += 1;
        }
    }
    (*owner.state_ptr()).largest = (*owner.state_ptr()).max_cardinality;
    1 != 0
}

#[rustfmt::skip]
pub(crate) const OFF_DIAG: [i8; 64] = [
    0, -1, -1, -1, -1, -1, -1, -1,
    1,  0, -1, -1, -1, -1, -1, -1,
    1,  1,  0, -1, -1, -1, -1, -1,
    1,  1,  1,  0, -1, -1, -1, -1,
    1,  1,  1,  1,  0, -1, -1, -1,
    1,  1,  1,  1,  1,  0, -1, -1,
    1,  1,  1,  1,  1,  1,  0, -1,
    1,  1,  1,  1,  1,  1,  1,  0,
];

#[rustfmt::skip]
pub(crate) const TRIANGLE: [u8; 64] = [
    6, 0, 1, 2, 2, 1, 0, 6,
    0, 7, 3, 4, 4, 3, 7, 0,
    1, 3, 8, 5, 5, 8, 3, 1,
    2, 4, 5, 9, 9, 5, 4, 2,
    2, 4, 5, 9, 9, 5, 4, 2,
    1, 3, 8, 5, 5, 8, 3, 1,
    0, 7, 3, 4, 4, 3, 7, 0,
    6, 0, 1, 2, 2, 1, 0, 6,
];

#[rustfmt::skip]
pub(crate) const FLIP_DIAG: [u8; 64] = [
    0,  8, 16, 24, 32, 40, 48, 56,
    1,  9, 17, 25, 33, 41, 49, 57,
    2, 10, 18, 26, 34, 42, 50, 58,
    3, 11, 19, 27, 35, 43, 51, 59,
    4, 12, 20, 28, 36, 44, 52, 60,
    5, 13, 21, 29, 37, 45, 53, 61,
    6, 14, 22, 30, 38, 46, 54, 62,
    7, 15, 23, 31, 39, 47, 55, 63,
];

#[rustfmt::skip]
pub(crate) const LOWER: [u8; 64] = [
    28,  0,  1,  2,  3,  4,  5,  6,
     0, 29,  7,  8,  9, 10, 11, 12,
     1,  7, 30, 13, 14, 15, 16, 17,
     2,  8, 13, 31, 18, 19, 20, 21,
     3,  9, 14, 18, 32, 22, 23, 24,
     4, 10, 15, 19, 22, 33, 25, 26,
     5, 11, 16, 20, 23, 25, 34, 27,
     6, 12, 17, 21, 24, 26, 27, 35,
];

#[rustfmt::skip]
pub(crate) const DIAG: [u8; 64] = [
     0,  0,  0,  0,  0,  0,  0,  8,
     0,  1,  0,  0,  0,  0,  9,  0,
     0,  0,  2,  0,  0, 10,  0,  0,
     0,  0,  0,  3, 11,  0,  0,  0,
     0,  0,  0, 12,  4,  0,  0,  0,
     0,  0, 13,  0,  0,  5,  0,  0,
     0, 14,  0,  0,  0,  0,  6,  0,
    15,  0,  0,  0,  0,  0,  0,  7,
];

#[rustfmt::skip]
pub(crate) const FLAP: [u8; 64] = [
        0,  0,  0,  0,  0,  0,  0, 0,
        0,  6, 12, 18, 18, 12,  6, 0,
        1,  7, 13, 19, 19, 13,  7, 1,
        2,  8, 14, 20, 20, 14,  8, 2,
        3,  9, 15, 21, 21, 15,  9, 3,
        4, 10, 16, 22, 22, 16, 10, 4,
        5, 11, 17, 23, 23, 17, 11, 5,
        0,  0,  0,  0,  0,  0,  0, 0,
];

#[rustfmt::skip]
pub(crate) const PAWN_TWIST: [u8; 64] = [
         0,  0,  0,  0,  0,  0,  0,  0,
        47, 35, 23, 11, 10, 22, 34, 46,
        45, 33, 21,  9,  8, 20, 32, 44,
        43, 31, 19,  7,  6, 18, 30, 42,
        41, 29, 17,  5,  4, 16, 28, 40,
        39, 27, 15,  3,  2, 14, 26, 38,
        37, 25, 13,  1,  0, 12, 24, 36,
         0,  0,  0,  0,  0,  0,  0,  0,
];

#[rustfmt::skip]
pub(crate) const KK_IDX: [[i16; 64]; 10] = [
    [
        -1, -1, -1,  0,  1,  2,  3,  4,
        -1, -1, -1,  5,  6,  7,  8,  9,
        10, 11, 12, 13, 14, 15, 16, 17,
        18, 19, 20, 21, 22, 23, 24, 25,
        26, 27, 28, 29, 30, 31, 32, 33,
        34, 35, 36, 37, 38, 39, 40, 41,
        42, 43, 44, 45, 46, 47, 48, 49,
        50, 51, 52, 53, 54, 55, 56, 57,
    ],
    [
         58,  -1,  -1,  -1,  59,  60,  61,  62,
         63,  -1,  -1,  -1,  64,  65,  66,  67,
         68,  69,  70,  71,  72,  73,  74,  75,
         76,  77,  78,  79,  80,  81,  82,  83,
         84,  85,  86,  87,  88,  89,  90,  91,
         92,  93,  94,  95,  96,  97,  98,  99,
        100, 101, 102, 103, 104, 105, 106, 107,
        108, 109, 110, 111, 112, 113, 114, 115,
    ],
    [
        116, 117,  -1,  -1,  -1, 118, 119, 120,
        121, 122,  -1,  -1,  -1, 123, 124, 125,
        126, 127, 128, 129, 130, 131, 132, 133,
        134, 135, 136, 137, 138, 139, 140, 141,
        142, 143, 144, 145, 146, 147, 148, 149,
        150, 151, 152, 153, 154, 155, 156, 157,
        158, 159, 160, 161, 162, 163, 164, 165,
        166, 167, 168, 169, 170, 171, 172, 173,
    ],
    [
        174,  -1,  -1,  -1, 175, 176, 177, 178,
        179,  -1,  -1,  -1, 180, 181, 182, 183,
        184,  -1,  -1,  -1, 185, 186, 187, 188,
        189, 190, 191, 192, 193, 194, 195, 196,
        197, 198, 199, 200, 201, 202, 203, 204,
        205, 206, 207, 208, 209, 210, 211, 212,
        213, 214, 215, 216, 217, 218, 219, 220,
        221, 222, 223, 224, 225, 226, 227, 228,
    ],
    [
        229, 230,  -1,  -1,  -1, 231, 232, 233,
        234, 235,  -1,  -1,  -1, 236, 237, 238,
        239, 240,  -1,  -1,  -1, 241, 242, 243,
        244, 245, 246, 247, 248, 249, 250, 251,
        252, 253, 254, 255, 256, 257, 258, 259,
        260, 261, 262, 263, 264, 265, 266, 267,
        268, 269, 270, 271, 272, 273, 274, 275,
        276, 277, 278, 279, 280, 281, 282, 283,
    ],
    [
        284, 285, 286, 287, 288, 289, 290, 291,
        292, 293,  -1,  -1,  -1, 294, 295, 296,
        297, 298,  -1,  -1,  -1, 299, 300, 301,
        302, 303,  -1,  -1,  -1, 304, 305, 306,
        307, 308, 309, 310, 311, 312, 313, 314,
        315, 316, 317, 318, 319, 320, 321, 322,
        323, 324, 325, 326, 327, 328, 329, 330,
        331, 332, 333, 334, 335, 336, 337, 338,
    ],
    [
         -1,  -1, 339, 340, 341, 342, 343, 344,
         -1,  -1, 345, 346, 347, 348, 349, 350,
         -1,  -1, 441, 351, 352, 353, 354, 355,
         -1,  -1,  -1, 442, 356, 357, 358, 359,
         -1,  -1,  -1,  -1, 443, 360, 361, 362,
         -1,  -1,  -1,  -1,  -1, 444, 363, 364,
         -1,  -1,  -1,  -1,  -1,  -1, 445, 365,
         -1,  -1,  -1,  -1,  -1,  -1,  -1, 446,
    ],
    [
         -1,  -1,  -1, 366, 367, 368, 369, 370,
         -1,  -1,  -1, 371, 372, 373, 374, 375,
         -1,  -1,  -1, 376, 377, 378, 379, 380,
         -1,  -1,  -1, 447, 381, 382, 383, 384,
         -1,  -1,  -1,  -1, 448, 385, 386, 387,
         -1,  -1,  -1,  -1,  -1, 449, 388, 389,
         -1,  -1,  -1,  -1,  -1,  -1, 450, 390,
         -1,  -1,  -1,  -1,  -1,  -1,  -1, 451,
    ],
    [
        452, 391, 392, 393, 394, 395, 396, 397,
         -1,  -1,  -1,  -1, 398, 399, 400, 401,
         -1,  -1,  -1,  -1, 402, 403, 404, 405,
         -1,  -1,  -1,  -1, 406, 407, 408, 409,
         -1,  -1,  -1,  -1, 453, 410, 411, 412,
         -1,  -1,  -1,  -1,  -1, 454, 413, 414,
         -1,  -1,  -1,  -1,  -1,  -1, 455, 415,
         -1,  -1,  -1,  -1,  -1,  -1,  -1, 456,
    ],
    [
        457, 416, 417, 418, 419, 420, 421, 422,
         -1, 458, 423, 424, 425, 426, 427, 428,
         -1,  -1,  -1,  -1,  -1, 429, 430, 431,
         -1,  -1,  -1,  -1,  -1, 432, 433, 434,
         -1,  -1,  -1,  -1,  -1, 435, 436, 437,
         -1,  -1,  -1,  -1,  -1, 459, 438, 439,
         -1,  -1,  -1,  -1,  -1,  -1, 460, 440,
         -1,  -1,  -1,  -1,  -1,  -1,  -1, 461,
    ],
];

pub(crate) const FILE_TO_FILE: [u8; 8] = [0, 1, 2, 3, 3, 2, 1, 0];
const WDL_TO_MAP: [i32; 5] = [1, 3, 0, 2, 0];
const PA_FLAGS: [u8; 5] = [8, 0, 0, 0, 4];

pub(crate) struct Indices {
    pub(crate) binomial: [[u64; 64]; 7],
    pub(crate) pawn_idx: [[u64; 24]; 6],
    pub(crate) pawn_factor_file: [[u64; 4]; 6],
}

pub(crate) static INDICES: Indices = generate_indices();

const fn generate_indices() -> Indices {
    let mut binomial = [[0; 64]; 7];
    let mut pawn_idx = [[0; 24]; 6];
    let mut pawn_factor_file = [[0; 4]; 6];
    let mut i = 0;
    while i < 7 {
        let mut j = 0;
        while j < 64 {
            if j >= i {
                let mut f = 1u64;
                let mut l = 1u64;
                let mut k = 0;
                while k < i {
                    f *= (j - k) as u64;
                    l *= (k + 1) as u64;
                    k += 1;
                }
                binomial[i][j] = f / l;
            }
            j += 1;
        }
        i += 1;
    }
    i = 0;
    while i < 6 {
        let mut sum = 0u64;
        let mut j = 0;
        while j < 24 {
            pawn_idx[i][j] = sum;
            sum += binomial[i][PAWN_TWIST[(1 + j % 6) * 8 + j / 6] as usize];
            if (j + 1) % 6 == 0 {
                pawn_factor_file[i][j / 6] = sum;
                sum = 0;
            }
            j += 1;
        }
        i += 1;
    }
    Indices {
        binomial,
        pawn_idx,
        pawn_factor_file,
    }
}

#[cfg(test)]
mod index_tests {
    use super::INDICES;

    fn fingerprint<const ROWS: usize, const COLS: usize>(array: &[[u64; COLS]; ROWS]) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for row in array {
            for value in row {
                for byte in value.to_le_bytes() {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
                }
            }
        }
        hash
    }

    #[test]
    fn immutable_indices_match_the_frozen_full_arrays() {
        // Fingerprints cover every entry of the previous runtime arrays.
        assert_eq!(fingerprint(&INDICES.binomial), 0x1471afc56d4ec979);
        assert_eq!(fingerprint(&INDICES.pawn_idx), 0x891bbb4aa8a1a815);
        assert_eq!(fingerprint(&INDICES.pawn_factor_file), 0x7b5d8490b32599d0);
        assert_eq!(INDICES.binomial[6][63], 67_945_521);
        assert_eq!(INDICES.pawn_idx[5][23], 610);
        assert_eq!(INDICES.pawn_factor_file[5][3], 610);
    }
}

fn load_table(
    owner: &Generation,
    pos: *const PyrrhicPosition,
    original: *const BaseEntry,
    key: u64,
    kind: TableType,
) -> Result<LoadedTable, LoadError> {
    let mut name = [0 as c_char; 16];
    unsafe {
        prt_str(pos, name.as_mut_ptr(), ((*original).key != key) as i32);
        let file = open_tb(owner, name.as_ptr(), TB_SUFFIX[kind as usize])
            .map_err(|_| LoadError::MissingOrInvalid)?;
        let size = file
            .metadata()
            .map_err(|_| LoadError::MissingOrInvalid)?
            .len();
        if size < TB_MIN_FILE_SIZE || size & 63 != 16 {
            return Err(LoadError::MissingOrInvalid);
        }
        let backing = TableBytes::map(&file).map_err(|_| LoadError::MissingOrInvalid)?;
        if backing.as_slice().len() as u64 != size {
            return Err(LoadError::MissingOrInvalid);
        }
        let (primary_pawns, secondary_pawns, king_pair) = if (*original).hasPawns {
            let pawns = (*original).c2rust_unnamed.pawns;
            (usize::from(pawns[0]), usize::from(pawns[1]), false)
        } else {
            (0, 0, (*original).c2rust_unnamed.kk_enc)
        };
        let description = Description {
            pieces: usize::from((*original).num),
            primary_pawns,
            secondary_pawns,
            king_pair,
        };
        let parsed = parse_table(
            backing.as_slice(),
            TB_MAGIC[kind as usize],
            kind == TableType::Wdl,
            description,
            &INDICES.pawn_factor_file,
        )
        .map_err(|_| LoadError::MissingOrInvalid)?;
        let mut table = LoadedTable::new(original, description);
        table.install(backing, parsed);
        Ok(table)
    }
}

pub(crate) unsafe fn probe_table(
    owner: &Generation,
    mut pos: *const PyrrhicPosition,
    mut s: i32,
    mut success: *mut i32,
    type_0: i32,
) -> i32 {
    let state = owner.state_ptr();
    let mut key: u64 = pyrrhic_calc_key(pos, 0);
    if type_0 == WDL as i32 && key == 0 {
        return 0;
    }
    let mut hashIdx: i32 = (key >> (64 - 12)) as i32;
    while (*state).hash[hashIdx as usize].key != 0 && (*state).hash[hashIdx as usize].key != key {
        hashIdx = (hashIdx + 1) & ((1 << 12) - 1);
    }
    let Some(entry) = (*state).hash[hashIdx as usize].entry else {
        *success = 0;
        return 0;
    };
    let original: *mut BaseEntry = match entry {
        EntryIndex::Piece(index) => &raw mut (*state).piece_entry[index].be,
        EntryIndex::Pawn(index) => &raw mut (*state).pawn_entry[index].be,
    };
    if type_0 == DTZ as i32 && !(*original).hasDtz {
        *success = 0;
        return 0;
    }
    let kind = if type_0 == WDL as i32 {
        TableType::Wdl
    } else {
        TableType::Dtz
    };
    let table = match (*original).loaded[type_0 as usize]
        .get_or_init(|| load_table(owner, pos, original, key, kind))
    {
        Ok(table) => table,
        Err(_) => {
            *success = 0;
            return 0;
        }
    };
    let Some(parsed) = table.parsed.as_ref() else {
        *success = 0;
        return 0;
    };
    let Some(backing) = table.backing.as_ref() else {
        *success = 0;
        return 0;
    };
    let be = table.entry_ptr();
    let mut bside: bool = false;
    let mut flip: bool = false;
    if !(*be).symmetric {
        flip = key != (*be).key;
        bside = ((*pos).turn as i32 == PYRRHIC_WHITE as i32) as i32 == flip as i32;
    } else {
        flip = (*pos).turn as i32 != PYRRHIC_WHITE as i32;
        bside = false;
    }
    let description = table.description;
    let position = &*pos;
    let mut squares = [0u8; 7];
    let mut t: usize = 0;
    let mut flags: u8 = 0;
    let pair_index: usize;
    let idx: u64 = if !(*be).hasPawns {
        pair_index = if type_0 == WDL as i32 {
            usize::from(bside)
        } else {
            0
        };
        if type_0 == DTZ as i32 {
            let Some(pair) = parsed.pairs.first().and_then(Option::as_ref) else {
                *success = 0;
                return 0;
            };
            flags = pair.header.flags();
            if flags as i32 & 1 != bside as i32 && !(*be).symmetric {
                *success = -(1);
                return 0;
            }
        }
        let Some(encoding) = parsed.encodings.get(pair_index) else {
            *success = 0;
            return 0;
        };
        let mut filled = 0;
        while filled < description.pieces {
            let Ok(next) = fill_squares(
                position,
                encoding,
                description,
                flip,
                0,
                &mut squares,
                filled,
            ) else {
                *success = 0;
                return 0;
            };
            filled = next;
        }
        let Ok(value) = encode_squares(&mut squares, encoding, description) else {
            *success = 0;
            return 0;
        };
        value
    } else {
        let mirror = if flip { 0x38 } else { 0 };
        let Some(first) = parsed.encodings.first() else {
            *success = 0;
            return 0;
        };
        let Ok(mut filled) =
            fill_squares(position, first, description, flip, mirror, &mut squares, 0)
        else {
            *success = 0;
            return 0;
        };
        let Ok(file) = leading_pawn(&mut squares, description.primary_pawns) else {
            *success = 0;
            return 0;
        };
        t = file;
        pair_index = if type_0 == WDL as i32 {
            t + 4 * usize::from(bside)
        } else {
            t
        };
        if type_0 == DTZ as i32 {
            let Some(pair) = parsed.pairs.get(t).and_then(Option::as_ref) else {
                *success = 0;
                return 0;
            };
            flags = pair.header.flags();
            if flags as i32 & 1 != bside as i32 && !(*be).symmetric {
                *success = -(1);
                return 0;
            }
        }
        let Some(encoding) = parsed.encodings.get(pair_index) else {
            *success = 0;
            return 0;
        };
        while filled < description.pieces {
            let Ok(next) = fill_squares(
                position,
                encoding,
                description,
                flip,
                mirror,
                &mut squares,
                filled,
            ) else {
                *success = 0;
                return 0;
            };
            filled = next;
        }
        let Ok(value) = encode_squares(&mut squares, encoding, description) else {
            *success = 0;
            return 0;
        };
        value
    };
    let Some(pair) = parsed.pairs.get(pair_index).and_then(Option::as_ref) else {
        *success = 0;
        return 0;
    };
    if !parsed
        .encodings
        .get(pair_index)
        .is_some_and(|encoding| idx < encoding.size)
    {
        *success = 0;
        return 0;
    }
    let Ok(decoded) = decode_pair(backing.as_slice(), pair, idx) else {
        *success = 0;
        return 0;
    };
    if type_0 == WDL as i32 {
        return i32::from(decoded[0]) - 2;
    }
    let Some(wdl_index) = s
        .checked_add(2)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|&index| index < 5)
    else {
        *success = 0;
        return 0;
    };
    let mut v: i32 = i32::from(decoded[0]) + ((i32::from(decoded[1]) & 0xf) << 8);
    if flags as i32 & 2 != 0 {
        let category = WDL_TO_MAP[wdl_index] as usize;
        let Some(range) = parsed
            .map_ranges
            .get(t)
            .and_then(|ranges| ranges.get(category))
        else {
            *success = 0;
            return 0;
        };
        let Some(map) = backing.as_slice().get(range.clone()) else {
            *success = 0;
            return 0;
        };
        let Some(value_index) = usize::try_from(v).ok() else {
            *success = 0;
            return 0;
        };
        if flags & 16 == 0 {
            let Some(&value) = map.get(value_index) else {
                *success = 0;
                return 0;
            };
            v = i32::from(value);
        } else {
            let Some(offset) = value_index.checked_mul(2) else {
                *success = 0;
                return 0;
            };
            let Some(value) = map.get(offset..offset + 2) else {
                *success = 0;
                return 0;
            };
            v = i32::from(u16::from_le_bytes([value[0], value[1]]));
        }
    }
    if flags as i32 & PA_FLAGS[wdl_index] as i32 == 0 || s & 1 != 0 {
        v *= 2;
    }
    v
}
unsafe fn probe_wdl_table(
    owner: &Generation,
    mut pos: *const PyrrhicPosition,
    mut success: *mut i32,
) -> i32 {
    probe_table(owner, pos, 0, success, WDL as i32)
}
unsafe fn probe_dtz_table(
    owner: &Generation,
    mut pos: *const PyrrhicPosition,
    mut wdl: i32,
    mut success: *mut i32,
) -> i32 {
    probe_table(owner, pos, wdl, success, DTZ as i32)
}
unsafe fn probe_ab<E: EngineAdapter>(
    owner: &Generation,
    mut pos: *const PyrrhicPosition,
    mut alpha: i32,
    mut beta: i32,
    mut success: *mut i32,
) -> i32 {
    assert!((*pos).ep == 0);
    let moves = match generate_captures::<E>(&*pos) {
        Ok(moves) => moves,
        Err(_) => {
            *success = 0;
            return 0;
        }
    };
    for &move_0 in moves.as_slice() {
        let mut pos1: PyrrhicPosition = PyrrhicPosition {
            white: 0,
            black: 0,
            kings: 0,
            queens: 0,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: 0,
            rule50: 0,
            ep: 0,
            turn: false,
        };
        if pyrrhic_is_capture(pos, move_0) && pyrrhic_do_move::<E>(&mut pos1, pos, move_0) {
            let mut v: i32 = -probe_ab::<E>(owner, &pos1, -beta, -alpha, success);
            if *success == 0 {
                return 0;
            }
            if v > alpha {
                if v >= beta {
                    return v;
                }
                alpha = v;
            }
        }
    }
    let mut v_0: i32 = probe_wdl_table(owner, pos, success);
    if alpha >= v_0 {
        alpha
    } else {
        v_0
    }
}
unsafe fn probe_wdl<E: EngineAdapter>(
    owner: &Generation,
    mut pos: *mut PyrrhicPosition,
    mut success: *mut i32,
) -> i32 {
    *success = 1;
    let moves = match generate_captures::<E>(&*pos) {
        Ok(moves) => moves,
        Err(_) => {
            *success = 0;
            return 0;
        }
    };
    let mut bestCap: i32 = -3;
    let mut bestEp: i32 = -3;
    for &move_0 in moves.as_slice() {
        let mut pos1: PyrrhicPosition = PyrrhicPosition {
            white: 0,
            black: 0,
            kings: 0,
            queens: 0,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: 0,
            rule50: 0,
            ep: 0,
            turn: false,
        };
        if pyrrhic_is_capture(pos, move_0) && pyrrhic_do_move::<E>(&mut pos1, pos, move_0) {
            let mut v: i32 = -probe_ab::<E>(owner, &pos1, -2, -bestCap, success);
            if *success == 0 {
                return 0;
            }
            if v > bestCap {
                if v == 2 {
                    *success = 2;
                    return 2;
                }
                if !pyrrhic_is_en_passant(pos, move_0) {
                    bestCap = v;
                } else if v > bestEp {
                    bestEp = v;
                }
            }
        }
    }
    let mut v_0: i32 = probe_wdl_table(owner, pos, success);
    if *success == 0 {
        return 0;
    }
    if bestEp > bestCap {
        if bestEp > v_0 {
            *success = 2;
            return bestEp;
        }
        bestCap = bestEp;
    }
    if bestCap >= v_0 {
        *success = 1 + (bestCap > 0) as i32;
        return bestCap;
    }
    if bestEp > -3 && v_0 == 0 {
        let moves = match generate_moves::<E>(&*pos) {
            Ok(moves) => moves,
            Err(_) => {
                *success = 0;
                return 0;
            }
        };
        let has_non_ep_legal = moves.as_slice().iter().any(|&candidate| {
            !pyrrhic_is_en_passant(pos, candidate) && pyrrhic_legal_move::<E>(pos, candidate)
        });
        if !has_non_ep_legal && !pyrrhic_is_check::<E>(pos) {
            *success = 2;
            return bestEp;
        }
    }
    v_0
}
const WDL_TO_DTZ: [i32; 5] = [-1, -101, 0, 101, 1];
unsafe fn probe_dtz<E: EngineAdapter>(
    owner: &Generation,
    mut pos: *mut PyrrhicPosition,
    mut success: *mut i32,
) -> i32 {
    let mut wdl: i32 = probe_wdl::<E>(owner, pos, success);
    if *success == 0 {
        return 0;
    }
    if wdl == 0 {
        return 0;
    }
    if *success == 2 {
        return WDL_TO_DTZ[(wdl + 2) as usize];
    }
    let mut legal_moves: Option<MoveList<256>> = None;
    let mut pos1: PyrrhicPosition = PyrrhicPosition {
        white: 0,
        black: 0,
        kings: 0,
        queens: 0,
        rooks: 0,
        bishops: 0,
        knights: 0,
        pawns: 0,
        rule50: 0,
        ep: 0,
        turn: false,
    };
    if wdl > 0 {
        let moves = match generate_legal::<E>(&*pos) {
            Ok(moves) => moves,
            Err(_) => {
                *success = 0;
                return 0;
            }
        };
        for &move_0 in moves.as_slice() {
            if !(!pyrrhic_is_pawn_move(pos, move_0) || pyrrhic_is_capture(pos, move_0) as i32 != 0)
                && pyrrhic_do_move::<E>(&mut pos1, pos, move_0)
            {
                let mut v: i32 = -probe_wdl::<E>(owner, &mut pos1, success);
                if *success == 0 {
                    return 0;
                }
                if v == wdl {
                    assert!(wdl < 3);
                    return WDL_TO_DTZ[(wdl + 2) as usize];
                }
            }
        }
        legal_moves = Some(moves);
    }
    let mut dtz: i32 = probe_dtz_table(owner, pos, wdl, success);
    if *success >= 0 {
        return WDL_TO_DTZ[(wdl + 2) as usize] + (if wdl > 0 { dtz } else { -dtz });
    }
    let mut best: i32 = 0;
    if wdl > 0 {
        best = 2147483647;
    } else {
        best = WDL_TO_DTZ[(wdl + 2) as usize];
    }
    let moves = if let Some(moves) = legal_moves {
        moves
    } else {
        match generate_moves::<E>(&*pos) {
            Ok(moves) => moves,
            Err(_) => {
                *success = 0;
                return 0;
            }
        }
    };
    for &move_1 in moves.as_slice() {
        if !(pyrrhic_is_capture(pos, move_1) as i32 != 0
            || pyrrhic_is_pawn_move(pos, move_1) as i32 != 0)
            && pyrrhic_do_move::<E>(&mut pos1, pos, move_1)
        {
            let mut v_0: i32 = -probe_dtz::<E>(owner, &mut pos1, success);
            let mate = if v_0 == 1 {
                match pyrrhic_is_mate::<E>(&pos1) {
                    Ok(mate) => mate,
                    Err(_) => {
                        *success = 0;
                        return 0;
                    }
                }
            } else {
                false
            };
            if mate {
                best = 1;
            } else if wdl > 0 {
                if v_0 > 0 && (v_0 + 1) < best {
                    best = v_0 + 1;
                }
            } else if (v_0 - 1) < best {
                best = v_0 - 1;
            }
            if *success == 0 {
                return 0;
            }
        }
    }
    best
}

unsafe fn probe_root<E: EngineAdapter>(
    owner: &Generation,
    mut pos: *mut PyrrhicPosition,
    mut score: *mut i32,
    mut results: *mut u32,
) -> u16 {
    let mut success: i32 = 0;
    let mut dtz: i32 = probe_dtz::<E>(owner, pos, &mut success);
    if success == 0 {
        return 0;
    }
    let mut scores: [i16; 256] = [0; 256];
    let moves = match generate_moves::<E>(&*pos) {
        Ok(moves) => moves,
        Err(_) => return 0,
    };
    let len = moves.len();
    if !results.is_null() && len >= 256 {
        return 0;
    }
    let mut num_draw: u64 = 0;
    let mut j: u32 = 0;
    let mut i: u32 = 0;
    while (i as usize) < len {
        let mut pos1: PyrrhicPosition = PyrrhicPosition {
            white: 0,
            black: 0,
            kings: 0,
            queens: 0,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: 0,
            rule50: 0,
            ep: 0,
            turn: false,
        };
        let candidate = moves.as_slice()[i as usize];
        if !pyrrhic_do_move::<E>(&mut pos1, pos, candidate) {
            scores[i as usize] = 0x7fff;
        } else {
            let mut v: i32 = 0;
            let mate = if dtz > 0 {
                match pyrrhic_is_mate::<E>(&pos1) {
                    Ok(mate) => mate,
                    Err(_) => return 0,
                }
            } else {
                false
            };
            if mate {
                v = 1;
            } else if pos1.rule50 as i32 != 0 {
                v = -probe_dtz::<E>(owner, &mut pos1, &mut success);
                if v > 0 {
                    v += 1;
                } else if v < 0 {
                    v -= 1;
                }
            } else {
                v = -probe_wdl::<E>(owner, &mut pos1, &mut success);
                v = WDL_TO_DTZ[(v + 2) as usize];
            }
            num_draw = num_draw.wrapping_add((v == 0) as i32 as u64);
            if success == 0 {
                return 0;
            }
            scores[i as usize] = v as i16;
            if !results.is_null() {
                let mut res: u32 = 0;
                res = res & !0xf | dtz_to_wdl((*pos).rule50 as i32, v) & 0xf;
                res = res & !0xfc00 | pyrrhic_move_from(candidate) << 10 & 0xfc00;
                res = res & !0x3f0 | pyrrhic_move_to(candidate) << 4 & 0x3f0;
                res = res & !0x70000 | pyrrhic_move_promotes(candidate) << 16 & 0x70000;
                res = res & !(0x80000)
                    | ((pyrrhic_is_en_passant(pos, candidate) as i32) << 19 & 0x80000) as u32;
                res =
                    res & !(0xfff00000) | ((if v < 0 { -v } else { v }) << 20) as u32 & 0xfff00000;
                let fresh29 = j;
                j = j.wrapping_add(1);
                *results.offset(fresh29 as isize) = res;
            }
        }
        i = i.wrapping_add(1);
    }
    if !results.is_null() {
        let fresh30 = j;
        j = j.wrapping_add(1);
        *results.offset(fresh30 as isize) = 0xffffffff;
    }
    if !score.is_null() {
        *score = dtz;
    }
    if dtz > 0 {
        let mut best: i32 = 0xffff;
        let mut best_move: u16 = 0;
        let mut i_0: u32 = 0;
        while (i_0 as usize) < len {
            let mut v_0: i32 = scores[i_0 as usize] as i32;
            if v_0 != 0x7fff as i32 && v_0 > 0 && v_0 < best {
                best = v_0;
                best_move = moves.as_slice()[i_0 as usize];
            }
            i_0 = i_0.wrapping_add(1);
        }
        (if best == 0xffff as i32 {
            0
        } else {
            best_move as i32
        }) as u16
    } else if dtz < 0 {
        let mut best_0: i32 = 0;
        let mut best_move_0: u16 = 0;
        let mut i_1: u32 = 0;
        while (i_1 as usize) < len {
            let mut v_1: i32 = scores[i_1 as usize] as i32;
            if v_1 != 0x7fff as i32 && v_1 < best_0 {
                best_0 = v_1;
                best_move_0 = moves.as_slice()[i_1 as usize];
            }
            i_1 = i_1.wrapping_add(1);
        }
        return (if best_0 == 0 {
            0xfffe as i32
        } else {
            best_move_0 as i32
        }) as u16;
    } else {
        if num_draw == 0 {
            return 0xffff as i32 as u16;
        }
        let mut count: u64 = (pyrrhic_calc_key(pos, !(*pos).turn as i32)).wrapping_rem(num_draw);
        let mut i_2: u32 = 0;
        while (i_2 as usize) < len {
            let mut v_2: i32 = scores[i_2 as usize] as i32;
            if v_2 != 0x7fff as i32 && v_2 == 0 {
                if count == 0 {
                    return moves.as_slice()[i_2 as usize];
                }
                count = count.wrapping_sub(1);
            }
            i_2 = i_2.wrapping_add(1);
        }
        return 0;
    }
}

#[cfg(test)]
mod initialization_tests {
    use super::*;

    #[test]
    fn owned_entries_and_unused_encoding_tails_are_initialized() {
        let dir = std::env::temp_dir().join(format!(
            "pyrrhic-initialized-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        for name in ["KQvK.rtbw", "KPvK.rtbw"] {
            File::create(dir.join(name)).unwrap().set_len(80).unwrap();
        }

        {
            let owner = Generation::new();
            assert!(unsafe { tb_init(&owner, dir.to_str().unwrap()) });
            let state = unsafe { &*owner.state_ptr() };
            assert!(state.num_piece > 0);
            assert!(state.num_pawn > 0);
            assert_eq!(state.piece_entry.len(), 650);
            assert_eq!(state.pawn_entry.len(), 861);
            for entry in &state.piece_entry {
                assert!(entry.be.loaded.iter().all(|cell| cell.get().is_none()));
            }
            for entry in &state.pawn_entry {
                assert!(entry.be.loaded.iter().all(|cell| cell.get().is_none()));
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
