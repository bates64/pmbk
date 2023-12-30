use std::collections::VecDeque;
use std::io::prelude::*;
use std::io::{Cursor, SeekFrom, Result};

#[cfg(feature = "rodio")]
use rodio::Source;

use crate::Instrument;

#[derive(Debug, Clone)]
pub struct VadpcmDecoder {
    instrument: Instrument,
    state: [i32; 16],
    codebook: Vec<Vec<Vec<i32>>>,
    wav_data_pos: usize,
    output_buffer: VecDeque<f32>,
}

/// Hardcoded in ucode.
const ORDER: usize = 2;

impl VadpcmDecoder {
    pub fn new(instrument: Instrument) -> Result<Self> {
        let mut cursor = Cursor::new(instrument.wav_data.as_slice());

        // You can tell how many pages are used by a vadpcm file based on the highest value of the second half of 4 bits on every ninth byte
        let pages = {
            let mut max = 0;
            for i in 0..instrument.wav_data.len() / 9 {
                // https://github.com/depp/skelly64/blob/main/docs/docs/vadpcm/codec.md#audio-data-1
                let control = instrument.wav_data[i * 9];
                let _scale_factor = /* high 4 bits */ (control & 0xF0) >> 4;
                let predictor_index = /* low 4 bits */ control & 0xF;

                if predictor_index > max {
                    max = predictor_index;
                }
            }
            cursor.seek(SeekFrom::Start(0))?;
            max as usize + 1
        };
        assert!(pages > 0 && pages <= 8); // usually 1 or 2

        let codebook = readaifccodebook(&instrument.predictor_data, ORDER, pages)?;

        Ok(Self {
            instrument,
            state: [0; 16],
            codebook,
            wav_data_pos: 0,
            output_buffer: VecDeque::new(),
        })
    }

    /// Reset back to the start of the file.
    pub fn reset(&mut self) {
        self.state = [0; 16];
        self.wav_data_pos = 0;
    }

    pub fn is_complete(&self) -> bool {
        self.wav_data_pos >= self.instrument.wav_data.len()
    }

    fn decode_frame(&mut self) {
        if self.is_complete() {
            self.output_buffer.push_back(0.0);
            return;
        }

        let frame = &self.instrument.wav_data[self.wav_data_pos..];
        vdecodeframe(frame, &mut self.state, ORDER, &self.codebook);
        self.wav_data_pos += 9;

        for sample in self.state.iter().copied() {
            // normalize to -1.0..1.0
            self.output_buffer.push_back(sample as f32 / i16::MAX as f32);
        }
    }

    fn decompressed_data_len(&self) -> usize {
        // each 9 bytes of wav_data become 16 bytes of decompressed data
        self.instrument.wav_data.len() * 16 / 9
    }
}

impl Iterator for VadpcmDecoder {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_complete() {
            return None;
        }
        if self.output_buffer.is_empty() {
            self.decode_frame();
        }
        self.output_buffer.pop_front()
    }
}

#[cfg(feature = "rodio")]
impl Source for VadpcmDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        // channels and sample rate are fixed
        None
    }

    fn channels(&self) -> u16 {
        1
    }

    fn sample_rate(&self) -> u32 {
        self.instrument.output_rate as u32
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        if self.instrument.has_loop() {
            None
        } else {
            // TODO: check this is correct
            let duration_ns = 1_000_000_000u64.checked_mul(self.decompressed_data_len() as u64).unwrap()
                / self.sample_rate() as u64
                / self.channels() as u64;
            let duration = std::time::Duration::new(
                duration_ns / 1_000_000_000,
                (duration_ns % 1_000_000_000) as u32,
            );
            Some(duration)
        }
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
    frame: &[u8], // should be 9 bytes
    outp: &mut [i32],
    order: usize,
    coef_table: &Vec<Vec<Vec<i32>>>,
) {
    let mut in_vec = [0; 16];
    let mut ix = [0; 16];

    let maxlevel = 7;
    let header = frame[0];
    let scale = 1 << (header >> 4);
    let optimalp = header & 0xF;

    let mut i = 0;
    while i < 16 {
        let c = frame.get(i / 2 + 1).copied().unwrap_or(0);
        ix[i] = (c >> 4) as i32;
        ix[i + 1] = (c & 0xF) as i32;

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
