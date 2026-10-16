// The only hand-written JS: hosts dsp.wasm (raw exports, no imports) on the audio thread.
class Dsp extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.w = new WebAssembly.Instance(new WebAssembly.Module(options.processorOptions.bytes), {}).exports;
    this.w.init(sampleRate);
    this.step = -1;
    this.port.onmessage = ({ data: [kind, a, cells] }) => {
      if (kind === "play") return this.w.set_playing(a);
      this.w.set_bpm(a);
      cells.forEach((on, i) => this.w.set_cell(i, on));
    };
  }

  process(_inputs, [out]) {
    const n = out[0].length;
    const step = this.w.process(n);
    const buf = new Float32Array(this.w.memory.buffer, this.w.out_ptr(), n);
    for (const ch of out) ch.set(buf);
    if (step !== this.step) this.port.postMessage((this.step = step));
    return true;
  }
}
registerProcessor("dsp", Dsp);
