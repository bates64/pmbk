use binrw::{binrw, BinRead, BinWrite, BinResult};
use binrw::file_ptr::parse_from_iter;

use std::error::Error;
use std::io::SeekFrom;

use clap::Parser;

mod vadpcm;

#[cfg(feature = "rodio")]
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

impl Instrument {
    pub fn has_loop(&self) -> bool {
        self.loop_end != 0
    }
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

    for (i, instrument) in bk.instruments.into_iter().enumerate() {
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

        let sample_rate = instrument.output_rate as u32;
        let decoder = vadpcm::VadpcmDecoder::new(instrument)?;

        let pcm = decoder.clone().into_iter().collect();
        let mut file = std::fs::File::create(format!("data/{}_{}.wav", bk.name, i))?;
        wav::write(wav::Header::new(wav::header::WAV_FORMAT_IEEE_FLOAT, 1, sample_rate, 32), &wav::BitDepth::ThirtyTwoFloat(pcm), &mut file)?;

        #[cfg(feature = "rodio")]
        playback::play(decoder);
    }

    Ok(())
}
