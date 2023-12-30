use std::io::prelude::*;
use std::io::{Cursor, SeekFrom, Result, ErrorKind};

use crate::Instrument;

pub struct VadpcmDecoder<'instr> {
    instrument: &'instr Instrument,
    state: [i32; 16],
    codebook: Vec<Vec<Vec<i32>>>,
    cursor: Cursor<&'instr [u8]>,
}

/// Hardcoded in ucode.
const ORDER: usize = 2;

impl<'instr> VadpcmDecoder<'instr> {
    pub fn new(instrument: &'instr Instrument) -> Result<Self> {
        let mut cursor = Cursor::new(instrument.wav_data.as_slice());

        // You can tell how many pages are used by a vadpcm file based on the highest value of the second half of 4 bits on every ninth byte
        let pages = {
            let mut max = 0;
            while cursor.position() + 8 < instrument.wav_data.len() as u64 {
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

        let codebook = readaifccodebook(&instrument.predictor_data, ORDER, pages)?;

        Ok(Self {
            instrument,
            state: [0; 16],
            codebook,
            cursor,
        })
    }

    /// Reset back to the start of the file.
    pub fn reset(&mut self) -> Result<()> {
        self.state = [0; 16];
        self.cursor.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    /// Decode the next sample.
    pub fn next_sample(&mut self) -> Result<[i16; 16]> {
        // TODO: check loop

        vdecodeframe(&mut self.cursor, &mut self.state, ORDER, &self.codebook)?;

        // clamp to 16-bit range
        let mut pcm = [0; 16];
        for i in 0..16 {
            pcm[i] = self.state[i].clamp(i16::MIN.into(), i16::MAX.into()) as i16;
        }
        Ok(pcm)
    }

    /// Decode all remaining samples until EOF.
    pub fn remaining_samples(&mut self) -> Result<Vec<i16>> {
        let mut samples = Vec::new();
        loop {
            match self.next_sample() {
                Ok(pcm) => samples.extend_from_slice(&pcm),
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
        }
        Ok(samples)
    }
}

fn readaifccodebook(
    data: &[i16],
    order: usize,
    npredictors: usize,
) -> Result<Vec<Vec<Vec<i32>>>> {
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

fn vdecodeframe(
    ifile: &mut Cursor<&[u8]>,
    outp: &mut [i32],
    order: usize,
    coef_table: &Vec<Vec<Vec<i32>>>,
) -> Result<()> {
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
