//! Two things done before the program is compiled.
//!
//! The mark is decoded into raw pixels, for the window's icon and for the corner of the window it is drawn
//! in, so the finished program carries pixels rather than a PNG decoder it would use once.
//!
//! And on Windows, the mark goes into the .exe itself, with a version resource beside it -- Explorer, the
//! taskbar's pinned button and Alt+Tab read the icon from the file before the program has drawn anything, and
//! Task Manager names the process by the version resource's description. No resource compiler is needed for
//! either: the resources are a few tables, written here. MSVC's linker is given a .res file, which it takes
//! as it is. The GNU linker does not read .res files, so for it the same tables are wrapped in a COFF object
//! with a .rsrc section, which is what windres would have made.

use std::path::{Path, PathBuf};

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    println!("cargo:rerun-if-env-changed=NOCTORIUM_VERSION");

    mark(&out);

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources(&out);
    }
}

// ------------------------------------------------------------------------------------------------ the mark

/// The window wants 8-bit RGBA, and a 256-pixel square is plenty for a taskbar and for the corner of the
/// window, so the 512-pixel mark is halved here rather than carried at four times the size.
fn mark(out: &Path) {
    let source = "assets/noctorium-song-find.png";
    println!("cargo:rerun-if-changed={source}");

    let file = std::fs::File::open(source).expect("the mark is missing from assets/");
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .expect("the mark is not a PNG");
    let (width, height) = {
        let info = reader.info();
        assert!(
            matches!(info.color_type, png::ColorType::Rgba)
                && matches!(info.bit_depth, png::BitDepth::Eight),
            "the mark must be 8-bit RGBA, not {:?} at {:?}",
            info.color_type,
            info.bit_depth
        );
        assert_eq!(info.width, info.height, "the mark must be square");
        (info.width as usize, info.height as usize)
    };
    let mut pixels = vec![0u8; width * height * 4];
    reader
        .next_frame(&mut pixels)
        .expect("the mark could not be read");

    let (pixels, side) = if width >= 512 {
        (halve(&pixels, width), width / 2)
    } else {
        (pixels, width)
    };

    std::fs::write(out.join("mark.rgba"), &pixels).expect("could not write the decoded mark");
    std::fs::write(
        out.join("mark.rs"),
        format!("/// The mark is square; this is its side in pixels.\npub const MARK_SIDE: usize = {side};\n"),
    )
    .expect("could not write the mark's size");
}

/// Each two-by-two block averaged into one pixel, weighted by how opaque each of the four is, so the
/// transparent corners do not bleed black into the edge of the rounded square.
fn halve(pixels: &[u8], side: usize) -> Vec<u8> {
    let half = side / 2;
    let mut out = Vec::with_capacity(half * half * 4);
    for y in 0..half {
        for x in 0..half {
            let mut sum = [0u32; 3];
            let mut alpha = 0u32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let at = ((2 * y + dy) * side + 2 * x + dx) * 4;
                let a = pixels[at + 3] as u32;
                for (c, total) in sum.iter_mut().enumerate() {
                    *total += pixels[at + c] as u32 * a;
                }
                alpha += a;
            }
            for total in sum {
                // A block with nothing opaque in it has no colour to speak of.
                out.push((total + alpha / 2).checked_div(alpha).unwrap_or(0) as u8);
            }
            out.push(((alpha + 2) / 4) as u8);
        }
    }
    out
}

// ------------------------------------------------------------------------------------- Windows resources

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// English (United Kingdom), and the Unicode code page, which is what the version strings are written in.
const LANGUAGE: u16 = 0x0809;
const CODE_PAGE: u16 = 1200;

struct Resource {
    kind: u16,
    name: u16,
    /// Moveable, pure, discardable: what rc gives each kind, which nothing reads any more but is set anyway.
    flags: u16,
    data: Vec<u8>,
}

fn windows_resources(out: &Path) {
    let icon = "assets/noctorium-song-find.ico";
    println!("cargo:rerun-if-changed={icon}");
    let ico = std::fs::read(icon).expect("the icon is missing from assets/");
    let mut resources = icon_resources(&ico);
    resources.push(Resource {
        kind: RT_VERSION,
        name: 1,
        flags: 0x0030,
        data: version_info(),
    });

    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let file = if env == "msvc" {
        let path = out.join("noctorium-song-find.res");
        std::fs::write(&path, res_file(&resources)).expect("could not write the resources");
        path
    } else {
        // The machine type, and the relocation that makes an address relative to the image's start.
        let (machine, relocation) = match arch.as_str() {
            "x86_64" => (0x8664, 0x0003),
            "aarch64" => (0xAA64, 0x0002),
            "x86" => (0x014C, 0x0007),
            other => {
                println!(
                    "cargo:warning=no icon in the .exe: resources are not written for {other}"
                );
                return;
            }
        };
        let path = out.join("noctorium-song-find-resources.o");
        std::fs::write(&path, coff_object(&resources, machine, relocation))
            .expect("could not write the resources");
        path
    };
    // Only the program: a test binary has no use for an icon.
    println!("cargo:rustc-link-arg-bins={}", file.display());
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend(x.to_le_bytes());
}

fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend(x.to_le_bytes());
}

fn pad(v: &mut Vec<u8>, to: usize) {
    while !v.len().is_multiple_of(to) {
        v.push(0);
    }
}

/// An .ico is a directory and the images; in a program, each image is a resource of its own and the
/// directory another, which names them by number rather than by where they are in the file.
fn icon_resources(ico: &[u8]) -> Vec<Resource> {
    let word = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let dword = |at: usize| u32::from_le_bytes([ico[at], ico[at + 1], ico[at + 2], ico[at + 3]]);
    assert!(
        ico.len() > 6 && word(0) == 0 && word(2) == 1,
        "the icon is not an .ico"
    );
    let count = word(4) as usize;

    let mut resources = Vec::new();
    let mut group = Vec::new();
    u16le(&mut group, 0);
    u16le(&mut group, 1);
    u16le(&mut group, count as u16);
    for index in 0..count {
        let entry = 6 + 16 * index;
        let size = dword(entry + 8) as usize;
        let offset = dword(entry + 12) as usize;
        let id = (index + 1) as u16;
        resources.push(Resource {
            kind: RT_ICON,
            name: id,
            flags: 0x1010,
            data: ico[offset..offset + size].to_vec(),
        });
        // Width, height, colours, reserved, planes, bits and size are the same twelve bytes as in the file;
        // only the last field changes, from an offset to the image's resource number.
        group.extend(&ico[entry..entry + 12]);
        u16le(&mut group, id);
    }
    resources.push(Resource {
        kind: RT_GROUP_ICON,
        name: 1,
        flags: 0x1030,
        data: group,
    });
    resources
}

/// The version, from the release tag when the release build sets NOCTORIUM_VERSION, as the program's own
/// About line has it.
fn version() -> String {
    std::env::var("NOCTORIUM_VERSION")
        .ok()
        .map(|v| v.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            std::env::var("CARGO_PKG_VERSION").expect("cargo sets the package version")
        })
}

fn utf16(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for unit in text.encode_utf16().chain([0]) {
        u16le(&mut out, unit);
    }
    out
}

/// One node of a version resource: its length, its value's length, whether that value is text, its key, and
/// then the value and the nodes under it, each starting on a four-byte boundary.
fn node(key: &str, value: &[u8], value_length: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut v = vec![0, 0];
    u16le(&mut v, value_length);
    u16le(&mut v, text as u16);
    v.extend(utf16(key));
    pad(&mut v, 4);
    v.extend(value);
    for child in children {
        pad(&mut v, 4);
        v.extend(child);
    }
    let length = v.len() as u16;
    v[0..2].copy_from_slice(&length.to_le_bytes());
    v
}

fn string(key: &str, value: &str) -> Vec<u8> {
    let encoded = utf16(value);
    node(key, &encoded, (encoded.len() / 2) as u16, true, &[])
}

fn version_info() -> Vec<u8> {
    let version = version();
    let mut parts = version
        .split(['.', '-', '+'])
        .map(|p| p.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    let high = ((major as u32) << 16) | minor as u32;
    let low = (patch as u32) << 16;

    let mut fixed = Vec::new();
    for value in [
        0xFEEF04BD, // the signature
        0x0001_0000,
        high,
        low, // the file's version
        high,
        low,         // the product's
        0x3F,        // which flags mean anything
        0,           // and none of them set
        0x0004_0004, // Windows NT
        1,           // an application
        0,
        0,
        0,
    ] {
        u32le(&mut fixed, value);
    }

    let table = node(
        &format!("{LANGUAGE:04X}{CODE_PAGE:04X}"),
        &[],
        0,
        true,
        &[
            string("CompanyName", "Noctorium"),
            string("FileDescription", "Noctorium Song Find"),
            string("FileVersion", &version),
            string("InternalName", "noctorium-song-find"),
            string("LegalCopyright", "GPL-3.0"),
            string("OriginalFilename", "Noctorium Song Find.exe"),
            string("ProductName", "Noctorium Song Find"),
            string("ProductVersion", &version),
        ],
    );
    let strings = node("StringFileInfo", &[], 0, true, &[table]);
    let mut translation = Vec::new();
    u16le(&mut translation, LANGUAGE);
    u16le(&mut translation, CODE_PAGE);
    let var = node("Translation", &translation, 4, false, &[]);
    let vars = node("VarFileInfo", &[], 0, true, &[var]);
    node(
        "VS_VERSION_INFO",
        &fixed,
        fixed.len() as u16,
        false,
        &[strings, vars],
    )
}

/// The .res format: an empty resource first, as rc writes, then each resource with a header naming its
/// kind, its number and its language.
fn res_file(resources: &[Resource]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut entry = |kind: u16, name: u16, flags: u16, language: u16, data: &[u8]| {
        u32le(&mut out, data.len() as u32);
        u32le(&mut out, 32);
        for id in [kind, name] {
            u16le(&mut out, 0xFFFF);
            u16le(&mut out, id);
        }
        u32le(&mut out, 0);
        u16le(&mut out, flags);
        u16le(&mut out, language);
        u32le(&mut out, 0);
        u32le(&mut out, 0);
        out.extend(data);
        pad(&mut out, 4);
    };
    entry(0, 0, 0, 0, &[]);
    for r in resources {
        entry(r.kind, r.name, r.flags, LANGUAGE, &r.data);
    }
    out
}

/// The same resources as a COFF object: one .rsrc section holding the three levels of directory a program's
/// resources are kept in -- kind, number, language -- then a data entry for each, then the data. The data
/// entries hold addresses relative to the start of the finished image, which only the linker knows, so each
/// gets a relocation against the section.
fn coff_object(resources: &[Resource], machine: u16, relocation: u16) -> Vec<u8> {
    let mut order: Vec<usize> = (0..resources.len()).collect();
    // Directories are searched in order, so their entries have to be sorted.
    order.sort_by_key(|&i| (resources[i].kind, resources[i].name));
    let mut kinds: Vec<u16> = order.iter().map(|&i| resources[i].kind).collect();
    kinds.dedup();

    let directory = |entries: usize| 16 + 8 * entries;
    let mut at = directory(kinds.len());
    let mut kind_tables = Vec::new();
    for kind in &kinds {
        kind_tables.push(at);
        at += directory(
            order
                .iter()
                .filter(|&&i| resources[i].kind == *kind)
                .count(),
        );
    }
    let language_tables: Vec<usize> = order
        .iter()
        .map(|_| {
            let here = at;
            at += directory(1);
            here
        })
        .collect();
    let data_entries: Vec<usize> = order
        .iter()
        .map(|_| {
            let here = at;
            at += 16;
            here
        })
        .collect();
    let data: Vec<usize> = order
        .iter()
        .map(|&i| {
            at = (at + 7) & !7;
            let here = at;
            at += resources[i].data.len();
            here
        })
        .collect();

    let mut section = Vec::new();
    let table = |s: &mut Vec<u8>, entries: usize| {
        u32le(s, 0);
        u32le(s, 0);
        u16le(s, 0);
        u16le(s, 0);
        u16le(s, 0);
        u16le(s, entries as u16);
    };
    // The high bit of an entry's offset says it points at another directory rather than at a data entry.
    table(&mut section, kinds.len());
    for (kind, offset) in kinds.iter().zip(&kind_tables) {
        u32le(&mut section, *kind as u32);
        u32le(&mut section, 0x8000_0000 | *offset as u32);
    }
    for kind in &kinds {
        let mine: Vec<usize> = (0..order.len())
            .filter(|&n| resources[order[n]].kind == *kind)
            .collect();
        table(&mut section, mine.len());
        for n in mine {
            u32le(&mut section, resources[order[n]].name as u32);
            u32le(&mut section, 0x8000_0000 | language_tables[n] as u32);
        }
    }
    assert_eq!(
        section.len(),
        language_tables[0],
        "the directories are not where they were counted"
    );
    for entry in &data_entries {
        table(&mut section, 1);
        u32le(&mut section, LANGUAGE as u32);
        u32le(&mut section, *entry as u32);
    }
    assert_eq!(
        section.len(),
        data_entries[0],
        "the data entries are not where they were counted"
    );
    for (n, &i) in order.iter().enumerate() {
        // Where the data is, from the start of the section; the relocation adds where the section ends up.
        u32le(&mut section, data[n] as u32);
        u32le(&mut section, resources[i].data.len() as u32);
        u32le(&mut section, 0);
        u32le(&mut section, 0);
    }
    for (n, &i) in order.iter().enumerate() {
        section.resize(data[n], 0);
        section.extend(&resources[i].data);
    }
    pad(&mut section, 4);

    let header = 20 + 40;
    let relocations_at = header + section.len();
    let symbols_at = relocations_at + 10 * data_entries.len();
    let mut out = Vec::new();
    u16le(&mut out, machine);
    u16le(&mut out, 1); // one section
    u32le(&mut out, 0); // no timestamp, so the build is the same every time
    u32le(&mut out, symbols_at as u32);
    u32le(&mut out, 1); // one symbol, the section's own
    u16le(&mut out, 0);
    u16le(&mut out, 0);

    out.extend(b".rsrc\0\0\0");
    u32le(&mut out, 0);
    u32le(&mut out, 0);
    u32le(&mut out, section.len() as u32);
    u32le(&mut out, header as u32);
    u32le(&mut out, relocations_at as u32);
    u32le(&mut out, 0);
    u16le(&mut out, data_entries.len() as u16);
    u16le(&mut out, 0);
    u32le(&mut out, 0x4000_0040); // initialised data, readable

    out.extend(&section);
    for entry in &data_entries {
        u32le(&mut out, *entry as u32);
        u32le(&mut out, 0);
        u16le(&mut out, relocation);
    }

    out.extend(b".rsrc\0\0\0");
    u32le(&mut out, 0);
    u16le(&mut out, 1); // in the first section
    u16le(&mut out, 0);
    out.push(3); // static
    out.push(0);
    u32le(&mut out, 4); // an empty string table
    out
}
