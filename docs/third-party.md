# Third-party software

## vgpu

Habibi's localhost Memory Graph bundles `vgpu` 0.3.1 from https://github.com/vercel-labs/vgpu. The pinned version and package integrity are recorded by `package-lock.json`. The generated browser bundle carries a license banner referring to the adjacent served copy at `/assets/vgpu-LICENSE.txt` (`web/vgpu-LICENSE.txt`).

MIT License

Copyright (c) 2025 Vercel, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

The generated browser bundle is built locally with Vite from Habibi-authored TypeScript and WGSL. It has no runtime CDN or network dependency.

## fastembed and BGE Small English v1.5

Habibi uses `fastembed` 6.0.2 for local CPU ONNX embedding inference. The crate is licensed under Apache-2.0.

The semantic tool index uses the quantized `Qdrant/bge-small-en-v1.5-onnx-Q` artifact at immutable revision `52398278842ec682c6f32300af41344b1c0b0bb2`. Its model card declares Apache-2.0. The artifact derives from `BAAI/bge-small-en-v1.5`; the upstream FlagEmbedding project and released model declare the MIT License. Exact files, sizes, and SHA-256 digests are in `models/bge-small-en-v1.5-onnx-q.json`. The model is not distributed in Git or embedded in the Habibi binary. `habibi embeddings install` downloads and verifies it for local offline inference.

The Apache-2.0 license text is distributed at `models/LICENSE.Apache-2.0.txt`; the upstream BGE MIT license is distributed at `models/LICENSE.BGE-MIT.txt`.
