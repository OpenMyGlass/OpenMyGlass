package org.example.demo.nano;

public interface Holder {
    public static final class Box extends MessageNano {
        public Leaf leaf;

        public void writeTo(CodedOutputByteBufferNano out) throws IOException {
            if (this.leaf != null) {
                out.writeMessage(1, this.leaf);
            }
        }
    }
}
