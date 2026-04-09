class PcmRecorderProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();
        this.targetSampleRate = options.processorOptions?.targetSampleRate || 16000;
        this.sourceOffset = 0;
        this.inputBuffer = [];
        this.outputBuffer = [];
        this.outputChunkSize = Math.ceil(this.targetSampleRate / 31.25);
        this.port.onmessage = (event) => {
            if (event.data?.type === 'flush') { this.flush(); }
        };
    }
    process(inputs) {
        const inputChannel = inputs[0]?.[0];
        if (!inputChannel?.length) return true;
        for (let i = 0; i < inputChannel.length; i++) this.inputBuffer.push(inputChannel[i]);
        if (sampleRate === this.targetSampleRate) {
            for (let i = 0; i < this.inputBuffer.length; i++) this.outputBuffer.push(this.inputBuffer[i]);
            this.inputBuffer = [];
            this.sourceOffset = 0;
        } else {
            const ratio = sampleRate / this.targetSampleRate;
            while (this.sourceOffset + ratio < this.inputBuffer.length) {
                const index = Math.floor(this.sourceOffset);
                const nextIndex = Math.min(index + 1, this.inputBuffer.length - 1);
                const fraction = this.sourceOffset - index;
                this.outputBuffer.push(this.inputBuffer[index] + (this.inputBuffer[nextIndex] - this.inputBuffer[index]) * fraction);
                this.sourceOffset += ratio;
            }
            const consumed = Math.floor(this.sourceOffset);
            if (consumed > 0) { this.inputBuffer = this.inputBuffer.slice(consumed); this.sourceOffset -= consumed; }
        }
        this.flush(false);
        return true;
    }
    flush(flushAll = true) {
        while (this.outputBuffer.length >= this.outputChunkSize || (flushAll && this.outputBuffer.length > 0)) {
            const frameCount = flushAll ? Math.min(this.outputChunkSize, this.outputBuffer.length) : this.outputChunkSize;
            const chunk = this.outputBuffer.splice(0, frameCount);
            const pcm16 = new Int16Array(chunk.length);
            for (let i = 0; i < chunk.length; i++) {
                const sample = Math.max(-1, Math.min(1, chunk[i]));
                pcm16[i] = sample < 0 ? sample * 0x8000 : sample * 0x7fff;
            }
            this.port.postMessage(pcm16.buffer, [pcm16.buffer]);
        }
    }
}
registerProcessor('pcm-recorder-processor', PcmRecorderProcessor);
