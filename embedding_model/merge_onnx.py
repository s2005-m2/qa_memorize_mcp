import onnx

print("Loading model with external data...")
model = onnx.load("model_fp16.onnx", load_external_data=True)
print(f"Model loaded. Saving as single file...")
onnx.save(model, "model.onnx")
print("Done. Saved model.onnx")
