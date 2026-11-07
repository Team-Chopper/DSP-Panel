// Microphone tap of the real-time analyser: collects the samples of the first
// input channel in blocks of 4096 (one FFT hop) and hands them to the page,
// which runs the analysis in WebAssembly. The output stays silent.

const BLOCK = 4096;

class RtaTap extends AudioWorkletProcessor {
  constructor() {
    super();
    this.block = new Float32Array(BLOCK);
    this.fill = 0;
  }

  process(inputs) {
    const channels = inputs[0];
    if (channels && channels.length > 0) {
      const samples = channels[0];
      let i = 0;
      while (i < samples.length) {
        const n = Math.min(samples.length - i, BLOCK - this.fill);
        this.block.set(samples.subarray(i, i + n), this.fill);
        this.fill += n;
        i += n;
        if (this.fill === BLOCK) {
          this.port.postMessage(this.block, [this.block.buffer]);
          this.block = new Float32Array(BLOCK);
          this.fill = 0;
        }
      }
    }
    return true;
  }
}

registerProcessor('rta-tap', RtaTap);
