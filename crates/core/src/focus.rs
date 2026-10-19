use std::sync::Arc;

use alkove_engine::{Model, Session};

use crate::{stream_io, Error, FOCUS};

// keeps the closest voice, or the voice of a print; 16 kHz in, 16 kHz out, `FOCUS.lag` samples late
pub struct Focus {
    net: Session,
}

impl Focus {
    pub fn new(model: &Arc<Model>) -> Result<Focus, Error> {
        let mut net = Session::new(model.clone())?;
        stream_io(&net, "audio", FOCUS.chunk, "audio_out", FOCUS.chunk)?;
        if net.input_mut("print").map(|p| p.len()) != Some(FOCUS.print) {
            return Err(Error::Model("focus: no print input of 192"));
        }
        Ok(Focus { net })
    }

    // the model never learned a print changing mid-stream: a new one starts from a fresh state; a shorter one keeps the end of the last
    pub fn set_print(&mut self, embedding: Option<&[f32]>) {
        let p = self.net.input_mut("print").expect("checked");
        match embedding {
            Some(e) => p[..e.len()].copy_from_slice(e),
            None => p.fill(0.0),
        }
        self.net.clear_state();
    }

    pub fn reset(&mut self) {
        self.net.clear_state();
    }

    pub fn step(&mut self, chunk: &[f32], out: &mut [f32]) {
        self.net.input_mut("audio").expect("checked").copy_from_slice(chunk);
        self.net.run();
        out.copy_from_slice(self.net.output("audio_out").expect("checked"));
    }
}
