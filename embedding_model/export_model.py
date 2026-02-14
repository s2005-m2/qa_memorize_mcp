"""Re-export all-MiniLM-L12-v2 as a clean ONNX model for ORT."""
import sys
import io
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8', errors='replace')
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding='utf-8', errors='replace')

import os
os.environ["PYTHONIOENCODING"] = "utf-8"

from transformers import AutoModel, AutoTokenizer
import torch
import numpy as np

model_name = "sentence-transformers/all-MiniLM-L12-v2"
tokenizer = AutoTokenizer.from_pretrained(model_name)
model = AutoModel.from_pretrained(model_name)
model.eval()

# Dummy inputs
dummy = tokenizer("hello world", return_tensors="pt")
input_ids = dummy["input_ids"]
attention_mask = dummy["attention_mask"]

out_path = "embedding_model/model_ort.onnx"

# Use legacy exporter explicitly to avoid dynamo issues
torch.onnx.export(
    model,
    (input_ids, attention_mask),
    out_path,
    input_names=["input_ids", "attention_mask"],
    output_names=["last_hidden_state"],
    dynamic_axes={
        "input_ids": {0: "batch", 1: "seq"},
        "attention_mask": {0: "batch", 1: "seq"},
        "last_hidden_state": {0: "batch", 1: "seq"},
    },
    opset_version=17,
    do_constant_folding=True,
    dynamo=False,
)
print("Export complete:", out_path)

# Verify with ORT
import onnxruntime as ort
sess = ort.InferenceSession(out_path)
ids = np.array([[101, 7592, 2088, 102]], dtype=np.int64)
mask = np.array([[1, 1, 1, 1]], dtype=np.int64)
result = sess.run(None, {"input_ids": ids, "attention_mask": mask})
print(f"Output shape: {result[0].shape}, dtype: {result[0].dtype}")
print("Verification passed!")
