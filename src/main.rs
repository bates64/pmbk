use binrw::{binrw, BinRead, BinWrite};
use binrw::file_ptr::parse_from_iter;

use std::{error::Error, io::Cursor};
use std::io::SeekFrom;
use std::io::prelude::*;

mod vadpcm;

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
    dc_book_size: u16, // in bytes
    #[br(
        seek_before(SeekFrom::Start(base as u64)),
        restore_position,
        count = dc_book_size as usize / std::mem::size_of::<i16>(),
        if(predictor != 0)
    )]
    predictor_data: Vec<i16>,

    key_base: u16, // pitch stuff

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

            for (i, instrument) in bk.instruments.iter().enumerate() {
                println!("  {:?} sample rate {} book size {}", instrument.r#type, instrument.output_rate, instrument.dc_book_size);

                let pcm = decode_vadpcm(instrument.wav_data.as_ref().unwrap(), &instrument.predictor_data)?;

                // sanity check that all samples arent zero
                let mut all_zero = true;
                for sample in &pcm {
                    if *sample != 0 {
                        all_zero = false;
                        break;
                    }
                }
                if all_zero {
                    panic!("all samples are zero");
                }

                // write to file
                // you can load this in audacity with the following settings:
                // Signed 16-bit PCM
                // Little-endian
                // 1 channel (mono)
                // {instrument.output_rate} Hz
                let mut file = std::fs::File::create(format!("data/{}_{}.raw16", bk.name, i))?;
                for sample in pcm {
                    let sample = sample as i16;
                    file.write_all(&sample.to_le_bytes())?;
                }
            }
        }
    }

    Ok(())
}

fn readaifccodebook(data: &[i16], order: usize, npredictors: usize) -> Result<Vec<Vec<Vec<i32>>>, Box<dyn Error>> {
    let mut table = vec![vec![vec![0; order + 8]; 8]; npredictors];
    let mut pos = 0;

    // pad data
    let required_len = npredictors * order * 8;
    let mut data = data.to_vec();
    data.resize(required_len, 0);
    
    for i in 0..npredictors {
        for j in 0..order {
            for k in 0..8 {
                table[i][k][j] = data[pos] as i32;
                pos += 1;
            }
        }

        for k in 1..8 {
            table[i][k][order] = table[i][k - 1][order - 1];
        }

        table[i][0][order] = 1 << 11;

        for k in 1..8 {
            for j in 0..k {
                table[i][j][k + order] = 0;
            }

            for j in k..8 {
                table[i][j][k + order] = table[i][j - k][order];
            }
        }
    }

    Ok(table)
}

fn decode_vadpcm(wave: &Vec<u8>, predictor: &[i16]) -> Result<Vec<i32>, Box<dyn Error>> {
    let mut cursor = Cursor::new(wave.as_slice());
    let mut pcm = Vec::new();

    /*
    /* You can tell how many pages are used by a vadpcm file based on the highest value of the second half of 4 bits on every ninth byte */
    // iterate over every 9th byte and find the max
    let pages = {
        let mut max = 0;
        while cursor.position() + 8 < wave.len() as u64 {
            let mut data = [0; 9];
            cursor.read_exact(&mut data)?;

            // https://github.com/depp/skelly64/blob/main/docs/docs/vadpcm/codec.md#audio-data-1
            let control = data[0];
            let _scale_factor = /* high 4 bits */ (control & 0xF0) >> 4;
            let predictor_index = /* low 4 bits */ control & 0xF;

            if predictor_index > max {
                max = predictor_index;
            }
        }
        cursor.seek(SeekFrom::Start(0))?;
        max as usize
    };
    dbg!(pages);

    // size = order * pages * 4
    let size = predictor.len() * std::mem::size_of::<i16>();
    let order = size / (pages * 4);
    */

    let booksize = predictor.len() * std::mem::size_of::<i16>();
    let order = 2;
    let pages = booksize / 8;

    dbg!(pages, order);

    // convert predictor into coef table
    let coef_table = readaifccodebook(predictor, order, pages)?;

    while cursor.position() + 1 < wave.len() as u64 {
        let mut data = [0; 16];
        vdecodeframe(&mut cursor, &mut data, order, &coef_table)?;
        pcm.extend_from_slice(&data);
    }

    Ok(pcm)
}

fn vdecodeframe(ifile: &mut Cursor<&[u8]>, outp: &mut [i32], order: usize, coef_table: &Vec<Vec<Vec<i32>>>) -> Result<(), Box<dyn Error>> {
    let mut in_vec = [0; 16];
    let mut ix = [0; 16];
    let mut header = [0; 1];
    let mut c = [0; 1];

    let maxlevel = 7;
    ifile.read_exact(&mut header)?;
    let scale = 1 << (header[0] >> 4);
    let optimalp = header[0] & 0xF;

    let mut i = 0;
    while i < 16 {
        ifile.read_exact(&mut c)?;
        ix[i] = (c[0] >> 4) as i32;
        ix[i + 1] = (c[0] & 0xF) as i32;

        if ix[i] <= maxlevel {
            ix[i] *= scale;
        } else {
            ix[i] = (-0x10 - -ix[i]) * scale;
        }
        
        if ix[i + 1] <= maxlevel {
            ix[i + 1] *= scale;
        } else {
            ix[i + 1] = (-0x10 - -ix[i + 1]) * scale;
        }

        i += 2;
    }

    for j in 0..2 {
        for i in 0..8 {
            in_vec[i + order] = ix[j * 8 + i];
        }

        if j == 0 {
            for i in 0..order {
                in_vec[i] = outp[16 - order + i];
            }
        } else {
            for i in 0..order {
                in_vec[i] = outp[j * 8 - order + i];
            }
        }

        for i in 0..8 {
            outp[i + j * 8] = inner_product(order + 8, &coef_table[optimalp as usize][i], &in_vec);
        }
    }

    Ok(())
}

fn inner_product(length: usize, v1: &[i32], v2: &[i32]) -> i32 {
    let mut out: i32 = 0;
    for j in 0..length {
        out = out.overflowing_add(v1[j].overflowing_mul(v2[j]).0).0;
    }

    // Compute "out / 2^11", rounded down.
    let dout = out / (1 << 11);
    let fiout = dout * (1 << 11);
    if out - fiout < 0 {
        dout - 1
    } else {
        dout
    }
}
