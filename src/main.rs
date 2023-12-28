use binrw::{binrw, FilePtr, PosValue, BinRead, BinWrite};
use binrw::file_ptr::parse_from_iter;

use std::{error::Error, io::Cursor};
use std::io::SeekFrom;
use std::io::prelude::*;

/*
typedef struct BKHeader {
    /* 0x00 */ u16 signature; // 'BK'
    /* 0x02 */ char unk_02[2];
    /* 0x04 */ s32 size;
    /* 0x08 */ s32 name;
    /* 0x0C */ u16 format; // 'CR', 'DR', 'SR'
    /* 0x0E */ char unk_0E[2];
    /* 0x10 */ char unk_10[2];
    /* 0x12 */ u16 instruments[16];
    /* 0x32 */ u16 instrumetsSize;
    /* 0x34 */ u16 unkStartA;
    /* 0x36 */ u16 unkSizeA;
    /* 0x38 */ u16 predictorsStart;
    /* 0x3A */ u16 predictorsSize;
    /* 0x3C */ u16 unkStartB;
    /* 0x3E */ u16 unkSizeB;
} BKHeader; // size = 0x40
 */

 /*
 // partially ALWaveTable?
typedef struct Instrument {
    /* 0x00 */ u8* base;
    /* 0x04 */ u32 wavDataLength;
    /* 0x08 */ UNK_PTR loopPredictor;
    /* 0x0C */ s32 loopStart;
    /* 0x10 */ s32 loopEnd;
    /* 0x14 */ s32 loopCount;
    /* 0x18 */ u16* predictor;
    /* 0x1C */ u16 dc_bookSize;
    /* 0x1E */ u16 keyBase;
    /* 0x20 */ union {
                    f32 pitchRatio;
                    s32 outputRate;
               };
    /* 0x24 */ u8 type;
    /* 0x25 */ u8 unk_25;
    /* 0x26 */ s8 unk_26;
    /* 0x27 */ s8 unk_27;
    /* 0x28 */ s8 unk_28;
    /* 0x29 */ s8 unk_29;
    /* 0x2A */ s8 unk_2A;
    /* 0x2B */ s8 unk_2B;
    /* 0x2C */ EnvelopePreset* envelopes;
} Instrument; // size = 0x30; */

#[binrw]
#[brw(big, magic = b"BK  ")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bk {
    size: i32,

    #[br(map = |x: [u8; 4]| String::from_utf8_lossy(&x).to_string())]
    #[bw(map = |x: &String| x.clone().into_bytes())]
    name: String,

    format: Format,

    #[brw(pad_before = 4)]
    instrument_offsets: [u16; 16],
    instruments_size: u16,

    #[br(
        parse_with = parse_from_iter(instrument_offsets.iter().copied().filter(|&o| o != 0)),
        seek_before(SeekFrom::Start(0))
    )]
    instruments: Vec<Instrument>,
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
enum Format {
    #[brw(magic = b"CR")] Cr,
    #[brw(magic = b"DR")] Dr,
    #[brw(magic = b"SR")] Sr,
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
struct Instrument {
    base: u32, // file ptr
    wav_data_length: u32,

    #[br(
        seek_before(SeekFrom::Start(base as u64)),
        restore_position,
        count = wav_data_length,
        if(wav_data_length > 0 && base != 0)
    )]
    wav_data: Option<Vec<u8>>,

    loop_predictor: u32, // bank ptr
    loop_start: i32,
    loop_end: i32,
    loop_count: i32,
    
    predictor: u32, // bank ptr
    dc_book_size: u16,
    key_base: u16,

    output_rate: i32, // au_swizzle_BK_instruments converts to f32 pitch ratio at runtime by dividing by gSoundGlobals->outputRate

    r#type: InstrumentType,
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
#[brw(repr = u8)]
enum InstrumentType {
    Adpcm, // https://github.com/depp/skelly64/blob/main/docs/docs/vadpcm/codec.md
    Raw16, // uncompressed
}

fn main() -> Result<(), Box<dyn Error>> {
    // Open all the .bk files and look for raw16
    for entry in std::fs::read_dir("../../pmret/papermario/assets/us/audio")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().unwrap_or_default() == "bk" {
            //dbg!(path.file_name());

            let file = std::fs::File::open(&path)?;
            let mut bufreader = std::io::BufReader::new(file);
            let bk: Bk = Bk::read(&mut bufreader)?;

            println!("{} {:?}", bk.name, bk.size);

            for instrument in bk.instruments {
                println!("  {:?} {}", instrument.r#type, instrument.output_rate)
            }
        }
    }

    /*
    let data = include_bytes!("../../../pmret/papermario/assets/us/audio/C6_BTL1.bk");
    let bk: Bk = Bk::read(&mut Cursor::new(data))?;

    assert_eq!(bk.size, data.len() as i32, "size mismatch");

    let instrument = &bk.instruments[0];

    if let Some(wav_data) = &instrument.wav_data {
        // write wav data to file
        let mut wav_file = std::fs::File::create("instr.dat")?;
        wav_file.write_all(wav_data)?;
    }
    */
    

    Ok(())
}
