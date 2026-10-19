#!/usr/bin/env python3
"""Reconstructs proto2 schemas from protobuf-nano classes decompiled with jadx.

Reads the writeTo() method of every class extending MessageNano/ExtendableMessageNano to
recover field number, type and cardinality, and turns interfaces holding only int constants
into enums. The output is a schema to be verified, not a copy of the code.

Usage: nano2proto.py --out DIR File1.java [File2.java ...]
"""
import argparse
import os
import re
import sys

SCALARS = {
    "Int32": "int32", "UInt32": "uint32", "SInt32": "sint32", "Fixed32": "fixed32", "SFixed32": "sfixed32",
    "Int64": "int64", "UInt64": "uint64", "SInt64": "sint64", "Fixed64": "fixed64", "SFixed64": "sfixed64",
    "Bool": "bool", "String": "string", "Bytes": "bytes", "Float": "float", "Double": "double",
}

DECL_RE = re.compile(r"\b(class|interface)\s+(\w+)(\s+extends\s+([\w.]+))?[^{;]*\{")
FIELD_RE = re.compile(r"(?:public|private|protected)\s+(?!static\b)(?:final\s+)?([\w.]+(?:\[\])?)\s+(\w+)\s*(?:=[^;]*)?;")
CONST_RE = re.compile(r"static\s+final\s+int\s+(\w+)\s*=\s*(-?(?:0x[0-9a-fA-F]+|\d+))\s*;")
FOREACH_RE = re.compile(r"for\s*\(\s*([\w.]+(?:\[\])*)\s+(\w+)\s*:\s*this\.(\w+)\s*\)")
INDEXVAR_RE = re.compile(r"([\w.]+(?:\[\])*)\s+(\w+)\s*=\s*this\.(\w+)\[\w+\]\s*;")
WRITE_RE = re.compile(r"\b\w+\.write(\w+?)(NoTag)?\(\s*(?:(\d+)\s*,\s*)?([^;]*?)\)\s*;")


LITERAL_RE = re.compile(r"""/\*.*?\*/|//[^\n]*|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'""", re.S)


def strip_literals(src):
    """Drops comments and empties string/char literals so braces inside literals do not confuse the parser.

    Single pass, so "//" inside a string is not taken for a comment and quotes inside comments are ignored."""
    def repl(m):
        t = m.group(0)
        return '""' if t[0] == '"' else "' '" if t[0] == "'" else " "
    return LITERAL_RE.sub(repl, src)


def match_brace(text, open_idx):
    depth = 0
    for i in range(open_idx, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i
    raise ValueError("unbalanced braces")


def children(body):
    """Depth-0 class/interface declarations inside a body, plus the remaining text without them."""
    out, rest, i = [], [], 0
    while True:
        m = DECL_RE.search(body, i)
        if not m:
            rest.append(body[i:])
            break
        depth = body[:m.start()].count("{") - body[:m.start()].count("}")
        open_idx = m.end() - 1
        end = match_brace(body, open_idx)
        if depth != 0:
            # deeper declaration (e.g. anonymous class inside a method): keep it in the text
            rest.append(body[i:end + 1])
            i = end + 1
            continue
        rest.append(body[i:m.start()])
        out.append((m.group(1), m.group(2), m.group(4) or "", body[open_idx + 1:end]))
        i = end + 1
    return out, "".join(rest)


class Enum:
    def __init__(self, name, values):
        self.name, self.values = name, values


class Message:
    def __init__(self, name):
        self.name, self.fields, self.nested = name, [], []


def parse_container(body, path, registry, outer):
    items = []
    decls, _ = children(body)
    for kind, name, extends, inner in decls:
        full = f"{path}.{name}" if path else name
        if kind == "class" and extends.split(".")[-1] in ("MessageNano", "ExtendableMessageNano"):
            items.append(parse_message(name, inner, full, registry, outer))
        elif kind == "interface":
            consts = CONST_RE.findall(inner)
            if consts and not children(inner)[0]:
                items.append(Enum(name, [(n, int(v, 0)) for n, v in consts]))
            else:
                items.extend(parse_container(inner, path, registry, outer))
    return items


def parse_message(name, body, path, registry, outer):
    msg = Message(name)
    registry[(outer, path)] = msg
    msg.nested = parse_container(body, path, registry, outer)
    _, own = children(body)
    msg.decl_types = {fname: ftype for ftype, fname in FIELD_RE.findall(own)}
    m = re.search(r"void\s+writeTo\s*\([^)]*\)[^{]*\{", own)
    msg.raw_fields = parse_write_to(own[m.end() - 1:match_brace(own, m.end() - 1) + 1]) if m else []
    msg.path = path
    return msg


def parse_write_to(body):
    var_field, fields, pending_packed = {}, [], None
    events = []
    for rx, kind in ((FOREACH_RE, "foreach"), (INDEXVAR_RE, "indexvar"), (WRITE_RE, "write")):
        events += [(m.start(), kind, m) for m in rx.finditer(body)]
    for _, kind, m in sorted(events, key=lambda e: e[0]):
        if kind in ("foreach", "indexvar"):
            var_field[m.group(2)] = (m.group(3), m.group(1))
            continue
        wtype, notag, num, arg = m.group(1), m.group(2), m.group(3), m.group(4).strip()
        if wtype == "RawVarint32":
            pending_packed = int(arg) >> 3 if arg.isdigit() else pending_packed
            continue
        if notag:
            fm = re.match(r"this\.(\w+)(\[\w+\])?$", arg)
            field = fm.group(1) if fm else var_field.get(arg, (None,))[0]
            if pending_packed is not None and field and wtype in SCALARS:
                fields.append(dict(num=pending_packed, wtype=wtype, field=field, repeated=True, packed=True, vtype=None))
                pending_packed = None
            continue
        if num is None:
            continue
        repeated, vtype = False, None
        fm = re.match(r"this\.(\w+)(\[\w+\])?$", arg)
        if fm:
            field, repeated = fm.group(1), bool(fm.group(2))
        elif arg in var_field:
            (field, vtype), repeated = var_field[arg], True
        else:
            print(f"warning: field {num} dropped, unrecognized argument: write{wtype}({num}, {arg})", file=sys.stderr)
            continue
        fields.append(dict(num=int(num), wtype=wtype, field=field, repeated=repeated, packed=False, vtype=vtype))
    seen, out = set(), []
    for f in fields:
        if f["num"] not in seen:
            seen.add(f["num"])
            out.append(f)
    return out


def resolve(ref, outer, path, registry, outers):
    parts = [p for p in ref.replace("[]", "").split(".")]
    while parts and parts[0][:1].islower():
        parts = parts[1:]
    ref = ".".join(parts)
    scope = path.split(".")
    for i in range(len(scope), -1, -1):
        cand = ".".join(scope[:i] + [ref]) if i else ref
        if (outer, cand) in registry:
            return outer, cand
    if parts and parts[0] in outers:
        for cand in (".".join(parts[1:]), ".".join(parts)):
            if (parts[0], cand) in registry:
                return parts[0], cand
    return None


def pkg(outer):
    return "glass." + outer.lower()


def render_type(f, msg, outer, registry, outers, imports):
    if f["wtype"] in SCALARS:
        return SCALARS[f["wtype"]], ""
    if f["wtype"] != "Message":
        return "bytes", f"  // unknown write type: write{f['wtype']} [unverified]"
    ref = f["vtype"] or msg.decl_types.get(f["field"], "")
    target = resolve(ref, outer, msg.path, registry, outers) if ref else None
    if not target:
        return "bytes", f"  // unresolved type: {ref or '?'} [unverified]"
    t_outer, t_path = target
    if t_outer != outer:
        imports.add(f"{t_outer}.proto")
        return f".{pkg(t_outer)}.{t_path}", ""
    if t_path.startswith(msg.path + "."):
        return t_path[len(msg.path) + 1:], ""
    # protoc resolves the first component from the innermost scope outwards: qualify the name if a
    # nested type with that name would shadow the intended one
    first, scope = t_path.split(".")[0], msg.path.split(".")
    for i in range(len(scope), -1, -1):
        prefix = ".".join(scope[:i])
        if (outer, f"{prefix}.{first}" if prefix else first) in registry:
            if prefix:
                return f".{pkg(outer)}.{t_path}", ""
            break
    return t_path, ""


def enum_hint(f, msg):
    if f["wtype"] not in ("Int32", "UInt32") or not f["field"]:
        return ""
    fname = f["field"].rstrip("_").lower()
    for e in msg.nested:
        if isinstance(e, Enum) and fname and fname in e.name.lower():
            return f"  // enum? {e.name} [unverified]"
    return ""


def render_enum(e, indent, taken):
    lines = [f"{indent}enum {e.name} {{"]
    vals = [v for _, v in e.values]
    if len(set(vals)) != len(vals):
        lines.append(f"{indent}  option allow_alias = true;")
    prefix = ""
    if any(n in taken for n, _ in e.values):
        prefix = re.sub(r"(?<!^)(?=[A-Z])", "_", e.name).upper() + "_"
    for n, v in e.values:
        lines.append(f"{indent}  {prefix}{n} = {v};")
        taken.add(prefix + n)
    lines.append(f"{indent}}}")
    return lines


def render_items(items, indent, outer, registry, outers, imports):
    blocks, taken = [], set()
    for it in items:
        if isinstance(it, Enum):
            blocks.append(render_enum(it, indent, taken))
        else:
            blocks.append(render_message(it, indent, outer, registry, outers, imports))
    out = []
    for i, b in enumerate(blocks):
        if i:
            out.append("")
        out.extend(b)
    return out


def render_message(msg, indent, outer, registry, outers, imports):
    lines = [f"{indent}message {msg.name} {{"]
    for f in msg.raw_fields:
        ptype, note = render_type(f, msg, outer, registry, outers, imports)
        label = "repeated" if f["repeated"] else "optional"
        opts = " [packed = true]" if f["packed"] else ""
        note = note or enum_hint(f, msg)
        lines.append(f"{indent}  {label} {ptype} {f['field'].rstrip('_')} = {f['num']}{opts};{note}")
    if msg.nested:
        lines.append("")
        lines.extend(render_items(msg.nested, indent + "  ", outer, registry, outers, imports))
    lines.append(f"{indent}}}")
    return lines


def parse_file(path, registry):
    src = strip_literals(open(path, encoding="utf-8", errors="replace").read())
    outer = os.path.splitext(os.path.basename(path))[0]
    m = re.search(r"\b(?:class|interface)\s+" + re.escape(outer) + r"\b(\s+extends\s+([\w.]+))?[^{]*\{", src)
    if not m:
        raise SystemExit(f"{path}: declaration '{outer}' not found")
    body = src[m.end():match_brace(src, m.end() - 1)]
    if (m.group(2) or "").split(".")[-1] in ("MessageNano", "ExtendableMessageNano"):
        # "one class per message" file: the file itself is the top-level message
        return outer, [parse_message(outer, body, outer, registry, outer)]
    return outer, parse_container(body, "", registry, outer)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("files", nargs="+")
    args = ap.parse_args()
    registry, parsed = {}, []
    for p in args.files:
        parsed.append((p,) + parse_file(p, registry))
    outers = {o for _, o, _ in parsed}
    os.makedirs(args.out, exist_ok=True)
    for p, outer, items in parsed:
        imports = set()
        body = render_items(items, "", outer, registry, outers, imports)
        head = [f"// Generated by tools/re/nano2proto.py from {os.path.basename(p)} — reconstructed schema, verify before use.",
                'syntax = "proto2";', "", f"package {pkg(outer)};", ""]
        if imports:
            head += [f'import "{i}";' for i in sorted(imports)] + [""]
        with open(os.path.join(args.out, f"{outer}.proto"), "w", encoding="utf-8") as fh:
            fh.write("\n".join(head + body) + "\n")
        n_msg = sum(1 for k in registry if k[0] == outer)
        print(f"{outer}.proto: {n_msg} messages")


if __name__ == "__main__":
    sys.exit(main())
