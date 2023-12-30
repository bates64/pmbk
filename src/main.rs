use binrw::{binrw, BinRead, BinWrite, BinResult};
use binrw::file_ptr::parse_from_iter;

use std::{error::Error, io::Cursor};
use std::io::SeekFrom;
use std::io::prelude::*;

use clap::Parser;

mod playback;

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
        seek_before(SeekFrom::Start(0)),
        restore_position,
    )]
    instruments: Vec<Instrument>,

    unk_start_a: u16,
    unk_size_a: u16,

    predictors_start: u16,
    predictors_size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
enum Format {
    #[brw(magic = b"CR")] Cr,
    #[brw(magic = b"DR")] Dr,
    #[brw(magic = b"SR")] Sr,
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
pub struct Instrument {
    base: u32, // file ptr
    wav_data_length: u32,
    #[br(
        seek_before(SeekFrom::Start(base as u64)),
        restore_position,
        count = wav_data_length,
        if(wav_data_length > 0 && base != 0)
    )]
    wav_data: Vec<u8>,

    loop_predictor: u32, // bank ptr
    loop_start: i32,
    loop_end: i32,
    loop_count: i32,
    #[br(
        seek_before(SeekFrom::Start(loop_predictor as u64)),
        restore_position,
        count = 16,
        if(loop_predictor != 0)
    )]
    loop_predictor_data: Vec<i16>,
    
    predictor: u32, // bank ptr
    dc_book_size: u16, // in bytes
    #[br(
        seek_before(SeekFrom::Start(predictor as u64)),
        restore_position,
        count = dc_book_size as usize / std::mem::size_of::<i16>(),
        if(predictor != 0)
    )]
    predictor_data: Vec<i16>,

    key_base: u16, // pitch stuff

    output_rate: i32, // au_swizzle_BK_instruments converts to f32 pitch ratio at runtime by dividing by gSoundGlobals->outputRate

    r#type: InstrumentType,

    #[br(pad_before = 7)]
    envelope_offset: u32,
    #[br(
        seek_before(SeekFrom::Start(envelope_offset as u64)),
        restore_position,
        if(envelope_offset != 0),
        parse_with = envelope_parser
    )]
    #[bw(ignore)]
    envelope: Envelope,
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
#[brw(repr = u8)]
pub enum InstrumentType {
    Adpcm, // https://github.com/depp/skelly64/blob/main/docs/docs/vadpcm/codec.md
    Raw16, // uncompressed
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about)]
/// BK file decoder. Writes decoded instruments to data/NAME_INDEX.wav
struct Args {
    /// BK file to read
    input: String,
}

// see au_update_voices
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Envelope {
    offsets: Vec<EnvelopeOffset>,
    cmds: Vec<EnvelopeCmd>,
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
pub struct EnvelopeOffset {
    press: u16,
    release: u16,
}

impl EnvelopeOffset {
    pub fn press_cmds<'a>(&self, cmds: &'a[EnvelopeCmd]) -> &'a[EnvelopeCmd] {
        let start = (self.press / 4) as usize;
        let mut end = start;
        for (i, cmd) in cmds[start..].iter().enumerate() {
            if let EnvelopeCmd::End(_) = cmd {
                end = start + i;
                break;
            }
        }
        &cmds[start..end]
    }

    pub fn release_cmds<'a>(&self, cmds: &'a[EnvelopeCmd]) -> &'a[EnvelopeCmd] {
        let start = (self.release / 4) as usize;
        let mut end = start;
        for (i, cmd) in cmds[start..].iter().enumerate() {
            if let EnvelopeCmd::End(_) = cmd {
                end = start + i;
                break;
            }
        }
        &cmds[start..end]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
pub enum EnvelopeCmd {
    #[br(magic = 0xFBu8)] EndLoop(u8),
    #[br(magic = 0xFCu8)] StartLoop { count: u8 }, // 0 means infinite
    #[br(magic = 0xFDu8)] AddMultiplier(u8),
    #[br(magic = 0xFEu8)] SetMultiplier(u8),
    #[br(magic = 0xFFu8)] End(u8),
    ChangeAmplitude {
        time: u8,  // index into AuEnvelopeIntervals, which are in microseconds
        amplitude: u8, // target amplitude to fade to
    },
}

#[binrw::parser(reader, endian)]
fn envelope_parser() -> BinResult<Envelope> {
    let count = u8::read_options(reader, endian, ())?;

    // 3 bytes padding
    reader.seek(SeekFrom::Current(3))?;

    // EnvelopeOffset x count
    let mut offsets = Vec::new();
    let mut max_offset = 0;
    for _ in 0..count {
        let offset = EnvelopeOffset::read_options(reader, endian, ())?;
        if offset.press > max_offset {
            max_offset = offset.press;
        }
        if offset.release > max_offset {
            max_offset = offset.release;
        }
        offsets.push(offset);
    }

    // read data up until offset=max_offset
    let mut cmds = Vec::new();
    for _ in 0..=(max_offset / 4) {
        cmds.push(EnvelopeCmd::read_options(reader, endian, ())?);
    }
    // keep reading until ENV_CMD_END
    loop {
        let cmd = EnvelopeCmd::read_options(reader, endian, ())?;
        if let EnvelopeCmd::End(_) = cmd {
            cmds.push(cmd);
            break;
        }
        cmds.push(cmd);
    }

    Ok(Envelope {
        offsets,
        cmds,
    })
}


fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let file = std::fs::File::open(&args.input)?;
    let mut bufreader = std::io::BufReader::new(file);
    let bk: Bk = Bk::read(&mut bufreader)?;

    println!("{} {:?}", bk.name, bk.size);

    let predictors_range = bk.predictors_start..(bk.predictors_start + bk.predictors_size);

    for (i, instrument) in bk.instruments.iter().enumerate() {
        println!("  {:?} sample rate {} book size {}", instrument.r#type, instrument.output_rate, instrument.dc_book_size);

        // loop info
        if instrument.loop_end != 0 {
            println!("    loop predictor {:#X}", instrument.loop_predictor);
            println!("    loop {:X}-{:X} {} times", instrument.loop_start, instrument.loop_end, instrument.loop_count);
        }

        // sanity check
        if !predictors_range.contains(&(instrument.predictor as u16)) {
            panic!("predictors {:#?} not in predictors block {:#?}", instrument.predictor, predictors_range);
        }

        let pcm = decode_vadpcm(&instrument.wav_data, &instrument.predictor_data)?;

        // repeat the looping part (TODO: this is probably wrong)
        let mut pcm = pcm.clone();
        let loop_count = if instrument.loop_count == -1 { 100 } else { instrument.loop_count }; // hack
        for _ in 0..loop_count {
            let loop_start = instrument.loop_start as usize;
            let loop_end = instrument.loop_end as usize;
            let loop_pcm = pcm[loop_start..loop_end].to_vec();

            // insert loop_pcm at loop_start
            pcm.splice(loop_start..loop_start, loop_pcm.iter().copied());

            // TODO: use loop_predictor_data somehow
        }

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

        playback::play(&instrument, &pcm);

        let mut file = std::fs::File::create(format!("data/{}_{}.wav", bk.name, i))?;
        wav::write(wav::Header::new(wav::header::WAV_FORMAT_PCM, 1, instrument.output_rate as u32, 16), &wav::BitDepth::Sixteen(pcm), &mut file)?;
    }

    Ok(())
}

fn readaifccodebook(data: &[i16], order: usize, npredictors: usize) -> Result<Vec<Vec<Vec<i32>>>, Box<dyn Error>> {
    let mut table = vec![vec![vec![0; order + 8]; 8]; npredictors];
    let mut pos = 0;

    // pad data
    let required_len = npredictors * order * 8;
    if required_len != data.len() {
        panic!("data len {} != required len {}", data.len(), required_len);
    }
    //let mut data = data.to_vec();
    //data.resize(required_len, 0);
    
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

fn decode_vadpcm(wave: &Vec<u8>, predictor: &[i16]) -> Result<Vec<i16>, Box<dyn Error>> {
    let mut cursor = Cursor::new(wave.as_slice());
    let mut pcm = Vec::new();

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
        max as usize + 1
    };
    let order = 2;

    dbg!(pages, order);

    // convert predictor into coef table
    let coef_table = readaifccodebook(predictor, order, pages)?;

    let mut data = [0; 16];
    while cursor.position() + 1 < wave.len() as u64 {
        vdecodeframe(&mut cursor, &mut data, order, &coef_table)?;

        // clamp to 16-bit range
        for i in 0..16 {
            pcm.push(data[i].clamp(i16::MIN.into(), i16::MAX.into()) as i16);
        }
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
        out += v1[j] * v2[j];
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
