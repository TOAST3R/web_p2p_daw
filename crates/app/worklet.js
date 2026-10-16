// The only hand-written JS: hosts dsp.wasm (raw exports, no imports) on the audio thread.
class Dsp extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.w = new WebAssembly.Instance(new WebAssembly.Module(options.processorOptions.bytes), {}).exports;
    this.w.init(sampleRate);
    this.port.onmessage = ({ data: [kind, a, b] }) => {
      if (kind === "anchor") return this.w.set_anchor(a, b);
      // "state": postcard bytes; view the buffer only after alloc (it may grow memory)
      new Uint8Array(this.w.memory.buffer, this.w.alloc(a.length), a.length).set(a);
      if (this.w.load_state(a.length) !== 0) console.warn("dsp: rejected malformed state");
    };
  }

  process(_inputs, [out]) {
    const n = out[0].length;
    this.w.process(n, currentFrame);
    const buf = new Float32Array(this.w.memory.buffer, this.w.out_ptr(), n);
    for (const ch of out) ch.set(buf);
    return true;
  }
}
registerProcessor("dsp", Dsp);
