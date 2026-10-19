package org.example.demo;

/* Synthetic fixture in decompiled protobuf-nano style (hand-written, not derived from Google code). */
public interface Demo {

    public static final class Envelope extends ExtendableMessageNano {
        private int bitField0_;
        public int version;
        private String name_;
        public Item[] items;
        private Other.Thing thing_;
        public long[] stamps;
        public String[] tags;
        public byte[] blob_;
        public Command command_;
        public byte[][] chunks;

        public void writeTo(CodedOutputByteBufferNano output) throws IOException {
            output.writeUInt32(1, this.version);
            if ((this.bitField0_ & 1) != 0) {
                output.writeString(2, this.name_);
            }
            if (this.items != null) {
                for (Item element : this.items) {
                    if (element != null) {
                        output.writeMessage(3, element);
                    }
                }
            }
            if (this.thing_ != null) {
                output.writeMessage(4, this.thing_);
            }
            if (this.stamps != null) {
                for (int i = 0; i < this.stamps.length; i++) {
                    output.writeInt64(5, this.stamps[i]);
                }
            }
            if (this.tags != null) {
                for (String element2 : this.tags) {
                    output.writeString(6, element2);
                }
            }
            output.writeBytes(7, this.blob_);
            if (this.command_ != null) {
                output.writeMessage(8, this.command_);
            }
            if (this.chunks != null) {
                for (byte[] element3 : this.chunks) {
                    output.writeBytes(9, element3);
                }
            }
            super.writeTo(output);
        }

        public static final class Item extends MessageNano {
            public float x;
            public boolean ok_;
            public String label = "a { tricky } string";

            public void writeTo(CodedOutputByteBufferNano codedOutputByteBufferNano) throws IOException {
                codedOutputByteBufferNano.writeFloat(1, this.x);
                codedOutputByteBufferNano.writeBool(2, this.ok_);
                codedOutputByteBufferNano.writeString(3, this.label);
            }
        }
    }

    public static final class Command extends MessageNano {
        public int command;
        public int[] flags;

        public interface CommandType {
            public static final int PING = 0;
            public static final int SCREENSHOT = 7;
        }

        public void writeTo(CodedOutputByteBufferNano output) throws IOException {
            output.writeInt32(1, this.command);
            if (this.flags != null && this.flags.length > 0) {
                output.writeRawVarint32(18);
                output.writeRawVarint32(this.flagsSize);
                for (int i = 0; i < this.flags.length; i++) {
                    output.writeInt32NoTag(this.flags[i]);
                }
            }
        }
    }
}
