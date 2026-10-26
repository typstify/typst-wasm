# typst-wasm

`typst-wasm` builds a wrapper around the Typst compiler and its IDE facilities into WebAssembly, so
you can edit and compile Typst sources in your browser!

## Features

1. Manage packages inside the wasm. Caller can add gzipped package tarball get from Typst Universe or [TPIX](https://tpix.typstify.com) and feed it
    to the engine.
2. Fonts management: Builtin fonts from Typst CLI is reserved. And custom fonts can also be used.
3. Three main export formats are supported: PDF, PNG, and SVG. Export/render range can be specified as well.
4. PDF standards and tag option are supported when rendering PDF.
5. IDE helper features: auto-completion, hover tips, source formatting, diagnostics, syntax highlighting, bi-directional jump between code and preview,

## Typst version

Only the latest released version of Typst is targeted.

## Fonts

To ensure basic typst documents render out of the box, this project embeds some but not all of the fonts from `typst-assets`, which takes up about 4Mb in size.

## Build

This project uses `wasm-pack` to build the wasm and generate related javascript glue code, so you have to install
`wasm-pack` first:

```
cargo install wasm-pack
```

You can use the `Makefile` directly to install `wasm-pack` and build the project. To build a package used with a javascript/ts bundler like Webpack:

```
make build
```

To build a package and use in web browsers as ES module natively:
```
make build-web
```

The original output wasm file is huge even after some wasm compiling optimization. To reduce the network load time, please consider compressing it
using GZIP or Brotli, the result size can be largely decreased:

| wasm file | size |
| --------  | ---- |
| Original | 27Mb |
| Gzipped  | 12Mb |
| Brotili compressed | 8.4Mb |

## Run the demo

The demo folder contains code to show basic features of this project. Please don't expect it to be a fully
funtionality editor! To run the demo, run:

```
npm install && npm run dev
```

## Cautions

This project is under heavy development, APIs may change as development continues. Please use it at your own risk.

