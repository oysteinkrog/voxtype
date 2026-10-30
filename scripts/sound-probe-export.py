#!/usr/bin/env python3
"""Make the two files the live worker's sound strip needs (ASR-10).

The sound strip shows which speech sounds (phones) the live model hears, a
little before it commits the words. A linear probe on one encoder layer gives
41 phone classes per 80 ms frame. This script writes, into the live model's
folder:

- encoder-sounds.onnx: the live encoder with that layer's output added as a
  second graph output. Weights and every other output are unchanged.
- sound-probe.bin: the probe, with its feature standardisation folded into
  the weights, in the format src/transcribe/sound_probe.rs reads.

Needs onnx, numpy and torch (the probe is a torch checkpoint with "layer",
"weight" (classes x dim), "bias", "mean", "std" and "phones").

usage: sound-probe-export.py PROBE.pt MODEL_DIR
"""

import struct
import sys
from pathlib import Path

import numpy as np
import onnx
import torch
from onnx import TensorProto, helper

MAGIC = b"VXSP"
VERSION = 1


def layer_output(layer):
    return f"/layers.{layer}/norm_out/LayerNormalization_output_0"


def export_encoder(model_dir, name):
    src = model_dir / "encoder.onnx"
    dst = model_dir / "encoder-sounds.onnx"
    m = onnx.load(str(src))
    produced = {o for node in m.graph.node for o in node.output}
    if name not in produced:
        sys.exit(f"{src} has no tensor {name}")
    if name not in {o.name for o in m.graph.output}:
        m.graph.output.append(
            helper.make_tensor_value_info(name, TensorProto.FLOAT, None)
        )
    onnx.save(m, str(dst))
    print(f"wrote {dst} with outputs {[o.name for o in m.graph.output]}")


def write_probe(ckpt, model_dir, name):
    w = ckpt["weight"].float().numpy().astype(np.float64)
    b = ckpt["bias"].float().numpy().astype(np.float64)
    mean = ckpt["mean"].float().numpy().astype(np.float64).reshape(-1)
    std = ckpt["std"].float().numpy().astype(np.float64).reshape(-1)
    phones = list(ckpt["phones"])
    classes, dim = w.shape
    assert b.shape == (classes,) and mean.shape == (dim,) and std.shape == (dim,)
    assert len(phones) == classes
    # logits = W (x - mean) / std + b = (W / std) x + (b - W (mean / std))
    wf = w / std
    bf = b - wf @ mean
    out = bytearray(MAGIC)
    out += struct.pack("<III", VERSION, dim, classes)
    for s in [name] + phones:
        enc = s.encode()
        out += struct.pack("<I", len(enc)) + enc
    out += wf.astype("<f4").tobytes()
    out += bf.astype("<f4").tobytes()
    dst = model_dir / "sound-probe.bin"
    dst.write_bytes(bytes(out))
    print(
        f"wrote {dst}: layer {ckpt['layer']}, {classes} classes x {dim}, {len(out)} bytes"
    )


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    ckpt = torch.load(sys.argv[1], map_location="cpu")
    model_dir = Path(sys.argv[2])
    name = layer_output(int(ckpt["layer"]))
    write_probe(ckpt, model_dir, name)
    export_encoder(model_dir, name)


if __name__ == "__main__":
    main()
