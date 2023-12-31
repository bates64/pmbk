use binrw::file_ptr::parse_from_iter;
use binrw::{binrw, BinRead, BinResult, BinWrite};

use std::io::{self, SeekFrom};

pub mod vadpcm;

#[cfg(feature = "rodio")]
pub mod playback;

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

    unk_start_a: u16,
    unk_size_a: u16,

    predictors_start: u16,
    predictors_size: u16,

    #[br(
        parse_with = parse_from_iter(instrument_offsets.iter().copied().filter(|&o| o != 0)),
        seek_before(SeekFrom::Start(0)),
        restore_position,
    )]
    instruments: Vec<Instrument>,
}

impl Bk {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn instruments(&self) -> &[Instrument] {
        &self.instruments
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
pub enum Format {
    #[brw(magic = b"CR")]
    Cr,
    #[brw(magic = b"DR")]
    Dr,
    #[brw(magic = b"SR")]
    Sr,
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

    predictor: u32,    // bank ptr
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

    // TODO: don't consume self
    #[cfg(feature = "wav")]
    pub fn write_wav<W: io::Write + io::Seek>(self, writer: &mut W) -> io::Result<()> {
        let sample_rate = self.output_rate as u32;

        let pcm = match self.r#type {
            InstrumentType::Adpcm => {
                let decoder = vadpcm::VadpcmDecoder::new(self);
                decoder.into_iter().collect()
            }
            InstrumentType::Raw16 => {
                // Convert to f32
                let mut pcm = Vec::new();
                for sample in self.wav_data.chunks_exact(2) {
                    let sample = i16::from_le_bytes([sample[0], sample[1]]);
                    pcm.push(sample as f32 / i16::MAX as f32);
                }
                pcm
            }
        };

        // We could write a 16-bit PCM wav, but for some instruments this doesn't sound right.
        // Instead, we write a 32-bit float wav because that works everywhere :)
        wav::write(
            wav::Header::new(wav::header::WAV_FORMAT_IEEE_FLOAT, 1, sample_rate, 32),
            &wav::BitDepth::ThirtyTwoFloat(pcm),
            writer,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BinRead, BinWrite)]
#[brw(repr = u8)]
pub enum InstrumentType {
    Adpcm, // https://github.com/depp/skelly64/blob/main/docs/docs/vadpcm/codec.md
    Raw16, // uncompressed, never used
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
    pub fn press_cmds<'a>(&self, cmds: &'a [EnvelopeCmd]) -> &'a [EnvelopeCmd] {
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

    pub fn release_cmds<'a>(&self, cmds: &'a [EnvelopeCmd]) -> &'a [EnvelopeCmd] {
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
    #[br(magic = 0xFBu8)]
    EndLoop(u8),
    #[br(magic = 0xFCu8)]
    StartLoop { count: u8 }, // 0 means infinite
    #[br(magic = 0xFDu8)]
    AddMultiplier(u8),
    #[br(magic = 0xFEu8)]
    SetMultiplier(u8),
    #[br(magic = 0xFFu8)]
    End(u8),
    ChangeAmplitude {
        time: u8,      // index into AuEnvelopeIntervals, which are in microseconds
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

    Ok(Envelope { offsets, cmds })
}
