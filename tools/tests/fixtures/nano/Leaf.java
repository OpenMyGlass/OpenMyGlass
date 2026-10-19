package org.example.demo.nano;

/* Synthetic fixture: a file that is itself a message (nano "one class per file" style). */
public final class Leaf extends ExtendableMessageNano<Leaf> {
    public String id;
    public Leaf.Part[] parts;

    public static final class Part extends MessageNano {
        public int size;

        public void writeTo(CodedOutputByteBufferNano out) throws IOException {
            out.writeInt32(1, this.size);
        }
    }

    public void writeTo(CodedOutputByteBufferNano out) throws IOException {
        out.writeString(1, this.id);
        if (this.parts != null && this.parts.length > 0) {
            for (int i = 0; i < this.parts.length; i++) {
                Part part = this.parts[i];
                if (part != null) {
                    out.writeMessage(2, part);
                }
            }
        }
    }
}
