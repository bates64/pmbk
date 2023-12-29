use modular_bitfield::prelude::*;
use binrw::BinRead;

use std::{error::Error, io::Cursor};
use std::io::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, BinRead)]
#[br(big)]
pub struct Frame {
  control: Control,
  residual: [u8; 8],
}

#[bitfield]
#[derive(Debug, Clone, PartialEq, Eq, BinRead)]
#[br(map = Self::from_bytes)]
pub struct Control {
  scaling_factor: B4,
  predictor_index: B4,
}

pub fn decode_vadpcm(frames: &[u8], codebook: &[i16]) -> Result<Vec<i32>, Box<dyn Error>> {
  // truncate frames to the nearest multiple of 9
  let frames = &frames[..frames.len() - (frames.len() % 9)]; // XXX
  dbg!(frames.len(), codebook.len());

  let frames = frames
    .chunks(9)
    .map(|chunk| Frame::read(&mut Cursor::new(chunk)))
    .collect::<Result<Vec<_>, _>>()?;

  let predictor_order = 2;
  let predictors = codebook.chunks(8).map(|chunk| {
    let mut predictor = [0; 8];
    predictor.copy_from_slice(chunk);
    predictor
  }).collect::<Vec<_>>();

  let mut pcm = Vec::new();

  let mut state = [0; 8];

  for frame in &frames {
    let mut accumulator = [0i32; 8];

    dbg!(frame.control.predictor_index());
    
    // add previous output to accumulator
    for i in 0..predictor_order {
      let previous_output = state[8 - predictor_order + i];
      for j in 0..8 {
        accumulator[j] = accumulator[j].overflowing_add((predictors[i][j] as i32).overflowing_mul(previous_output as i32).0).0;
      }
    }

    // calculate each output sample, and update the accumulator
    for i in 0..8 {
      let scaled_residual = (frame.residual[i] as i32) << frame.control.scaling_factor();
      let output = (accumulator[i] >> 11) + scaled_residual;
      for j in 0..7 - i {
        accumulator[i + 1 + j] = accumulator[i + 1 + j].overflowing_add(predictors[predictor_order - 1][j] as i32 * scaled_residual).0;
      }
      pcm.push(output);
    }

    // new decoder state is equal to the output
    for i in 0..8 {
      state[i] = pcm[pcm.len() - 8 + i];
    }
  }

  Ok(pcm)
}
