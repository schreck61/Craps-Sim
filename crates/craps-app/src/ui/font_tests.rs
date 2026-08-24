// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Compliance tests for the six embedded font subsets: total-size budget,
//! pinned content hashes, the OFL Reserved Font Name check (no source-font
//! names outside the copyright/license name records), and glyph coverage
//! for the typographic characters the product renders (−, ×, thin space,
//! ⌘). The TTF `name` and `cmap` tables are parsed directly — no font
//! dependencies.

use craps_engine::splitmix64;

/// The six embedded faces, exactly as `theme::install_fonts` registers them.
const FACES: [(&str, &[u8]); 6] = [
    (
        "LongrunDisplay-Medium",
        include_bytes!("../../assets/fonts/LongrunDisplay-Medium.ttf"),
    ),
    (
        "LongrunSans-Regular",
        include_bytes!("../../assets/fonts/LongrunSans-Regular.ttf"),
    ),
    (
        "LongrunSans-Medium",
        include_bytes!("../../assets/fonts/LongrunSans-Medium.ttf"),
    ),
    (
        "LongrunSans-SemiBold",
        include_bytes!("../../assets/fonts/LongrunSans-SemiBold.ttf"),
    ),
    (
        "LongrunMono-Regular",
        include_bytes!("../../assets/fonts/LongrunMono-Regular.ttf"),
    ),
    (
        "LongrunMono-Medium",
        include_bytes!("../../assets/fonts/LongrunMono-Medium.ttf"),
    ),
];

// ---------------------------------------------------------------------------
// Binary helpers
// ---------------------------------------------------------------------------

fn be_u16(b: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([b[off], b[off + 1]])
}

fn be_u32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Look up a top-level table by tag via the sfnt table directory.
fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let num_tables = be_u16(font, 4) as usize;
    for i in 0..num_tables {
        let rec = 12 + 16 * i;
        if &font[rec..rec + 4] == tag {
            let off = be_u32(font, rec + 8) as usize;
            let len = be_u32(font, rec + 12) as usize;
            return Some(&font[off..off + len]);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// a) Size budget
// ---------------------------------------------------------------------------

#[test]
fn total_size_under_ceiling() {
    let total: usize = FACES.iter().map(|(_, bytes)| bytes.len()).sum();
    assert!(
        total <= 900_000,
        "embedded fonts total {total} bytes — over the plan's 900 KB hard ceiling"
    );
}

// ---------------------------------------------------------------------------
// b) Pinned content hashes
// ---------------------------------------------------------------------------

/// Length-prefixed SplitMix64 fold over 8-byte little-endian chunks (the
/// last chunk zero-padded). Deterministic and dependency-free.
fn content_hash(bytes: &[u8]) -> u64 {
    let mut h = splitmix64(bytes.len() as u64);
    for chunk in bytes.chunks(8) {
        let mut buf = [0u8; 8];
        buf[..chunk.len()].copy_from_slice(chunk);
        h = splitmix64(h ^ u64::from_le_bytes(buf));
    }
    h
}

#[test]
fn content_hashes_pinned() {
    // These pins guard the checked-in font artifacts against silent
    // regeneration: any re-subset, re-instancing, or toolchain change that
    // touches a single byte fails here. A deliberate font update must
    // re-pin these values alongside the new files (and re-run the other
    // tests in this module against the new artifacts).
    const PINNED: [(&str, u64); 6] = [
        ("LongrunDisplay-Medium", 0x38F8_CFF5_4985_8A62),
        ("LongrunSans-Regular", 0x2676_597E_D587_FC2E),
        ("LongrunSans-Medium", 0x82E7_C344_6114_B944),
        ("LongrunSans-SemiBold", 0x72A8_8FD8_6FCE_C7BA),
        ("LongrunMono-Regular", 0x752F_8FAE_9833_10C9),
        ("LongrunMono-Medium", 0xF916_528F_6687_CBAF),
    ];
    for ((name, bytes), (pin_name, pin)) in FACES.iter().zip(PINNED) {
        assert_eq!(*name, pin_name, "face order drifted from the pin table");
        assert_eq!(
            content_hash(bytes),
            pin,
            "{name}.ttf content changed (hash {:#018X}) — if intentional, re-pin",
            content_hash(bytes)
        );
    }
}

// ---------------------------------------------------------------------------
// c) OFL Reserved Font Names
// ---------------------------------------------------------------------------

/// Decode every `name` table record as `(name_id, text)`. Platforms 0
/// (Unicode) and 3 (Windows) are UTF-16BE; platform 1 (Macintosh) is
/// treated as Latin-1, which is a superset of the MacRoman ASCII range the
/// check cares about.
fn name_records(font: &[u8]) -> Vec<(u16, String)> {
    let name = table(font, b"name").expect("font has no name table");
    let count = be_u16(name, 2) as usize;
    let string_off = be_u16(name, 4) as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let rec = 6 + 12 * i;
        let platform = be_u16(name, rec);
        let name_id = be_u16(name, rec + 6);
        let len = be_u16(name, rec + 8) as usize;
        let off = string_off + be_u16(name, rec + 10) as usize;
        let raw = &name[off..off + len];
        let text = match platform {
            0 | 3 => {
                let units: Vec<u16> = raw
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|p| u16::from_be_bytes(*p))
                    .collect();
                String::from_utf16_lossy(&units)
            }
            1 => raw.iter().map(|&b| b as char).collect(),
            _ => continue,
        };
        out.push((name_id, text));
    }
    out
}

#[test]
fn reserved_font_names_absent() {
    // OFL §1: renamed subsets must not use the Reserved Font Names of
    // their sources in any identifying name record. The copyright (0),
    // license (13), and license-URL (14) records may — and must — still
    // credit the sources; the trademark record (7) likewise carries the
    // required trademark attribution. Checked here are the records that
    // NAME the font: family (1), unique ID (3), full name (4), PostScript
    // name (6), and the typographic family/subfamily pair (16, 17).
    const CHECKED_IDS: [u16; 6] = [1, 3, 4, 6, 16, 17];
    const RESERVED: [&str; 3] = ["Fraunces", "Inter", "Plex"];
    for (face, bytes) in FACES {
        for (id, text) in name_records(bytes) {
            if !CHECKED_IDS.contains(&id) {
                continue;
            }
            for reserved in RESERVED {
                assert!(
                    !text.contains(reserved),
                    "{face}.ttf name record {id} contains reserved name {reserved:?}: {text:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// d) Glyph coverage
// ---------------------------------------------------------------------------

/// Map a codepoint through the font's best Unicode cmap subtable.
/// Returns 0 (.notdef) when unmapped.
fn cmap_glyph(font: &[u8], cp: u32) -> u16 {
    let cmap = table(font, b"cmap").expect("font has no cmap table");
    let n = be_u16(cmap, 2) as usize;
    let mut best: Option<usize> = None;
    let mut best_rank = 0u8;
    for i in 0..n {
        let rec = 4 + 8 * i;
        let rank = match (be_u16(cmap, rec), be_u16(cmap, rec + 2)) {
            (3, 10) | (0, 4) | (0, 6) => 3, // full Unicode repertoire
            (3, 1) | (0, 3) => 2,           // BMP
            (0, 0..=2) => 1,                // legacy Unicode
            _ => 0,
        };
        if rank > best_rank {
            best_rank = rank;
            best = Some(be_u32(cmap, rec + 4) as usize);
        }
    }
    let off = best.expect("font has no Unicode cmap subtable");
    let sub = &cmap[off..];
    match be_u16(sub, 0) {
        4 => lookup_format4(sub, cp),
        12 => lookup_format12(sub, cp),
        f => panic!("unsupported cmap subtable format {f}"),
    }
}

/// cmap format 4: segment mapping to delta values (BMP only).
fn lookup_format4(t: &[u8], cp: u32) -> u16 {
    let Ok(c) = u16::try_from(cp) else {
        return 0; // format 4 covers the BMP only
    };
    let seg_x2 = be_u16(t, 6) as usize;
    let ends = 14;
    let starts = ends + seg_x2 + 2; // +2 skips reservedPad
    let deltas = starts + seg_x2;
    let ranges = deltas + seg_x2;
    for i in 0..seg_x2 / 2 {
        let end = be_u16(t, ends + 2 * i);
        if end < c {
            continue;
        }
        let start = be_u16(t, starts + 2 * i);
        if start > c {
            return 0;
        }
        let delta = be_u16(t, deltas + 2 * i);
        let range_off = be_u16(t, ranges + 2 * i) as usize;
        if range_off == 0 {
            return c.wrapping_add(delta);
        }
        // idRangeOffset is relative to its own position in the table.
        let idx = ranges + 2 * i + range_off + 2 * (c - start) as usize;
        let g = be_u16(t, idx);
        return if g == 0 { 0 } else { g.wrapping_add(delta) };
    }
    0
}

/// cmap format 12: segmented coverage (full Unicode).
fn lookup_format12(t: &[u8], cp: u32) -> u16 {
    let n = be_u32(t, 12) as usize;
    for i in 0..n {
        let g = 16 + 12 * i;
        let (start, end) = (be_u32(t, g), be_u32(t, g + 4));
        if (start..=end).contains(&cp) {
            return (be_u32(t, g + 8) + (cp - start)) as u16;
        }
    }
    0
}

#[test]
fn glyph_coverage() {
    // Every face renders the honest minus (U+2212) and multiply (U+00D7)
    // signs the numeral system uses; the three Sans faces additionally
    // carry thin space (U+2009) and ⌘ (U+2318), which the mono and display
    // families reach through their Sans fallback (see install_fonts).
    for (face, bytes) in FACES {
        for cp in [0x2212u32, 0x00D7] {
            assert_ne!(
                cmap_glyph(bytes, cp),
                0,
                "{face}.ttf does not map U+{cp:04X}"
            );
        }
    }
    for (face, bytes) in FACES {
        if !face.starts_with("LongrunSans") {
            continue;
        }
        for cp in [0x2009u32, 0x2318] {
            assert_ne!(
                cmap_glyph(bytes, cp),
                0,
                "{face}.ttf does not map U+{cp:04X}"
            );
        }
    }
}
