# v1 includes the Blackfrost vision projector

The Local Model is a VLM. v1 downloads and can load `mmproj-Qwen3.8-27B-ABLITERATED-Q8_0.gguf` (629,247,488 bytes, SHA256 `6b8b9c95513e0798f4d48f467bced39491da62f418249bfb7ef12a857d15e5be`) with the Q4_K_M text weights. The F16 projector remains optional. Text-only launch is still possible by omitting `--mmproj` if VRAM is tight.
