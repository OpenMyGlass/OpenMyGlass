package org.example.demo;

public final class Other {
    public static final class Thing extends MessageNano {
        public double value;

        public void writeTo(CodedOutputByteBufferNano output) throws IOException {
            output.writeDouble(1, this.value);
        }
    }
}
