# Vendored highlight queries

These grammar crates ship a `highlights.scm` in their package but do not export it from
their Rust bindings, so fold includes a copy (§10.9). Each file is the crate's own query,
unmodified (files listed together are concatenated in order), under the crate's licence.

| File | From crate | Path in the crate | Licence |
|---|---|---|---|
| `ada.scm` | `tree-sitter-ada` 0.1.0 | `queries/highlights.scm` | MIT |
| `applesoft.scm` | `tree-sitter-applesoft` 5.0.0 | `queries/highlights.scm` | MIT |
| `caddy.scm` | `tree-sitter-caddy` 0.1.0 | `queries/highlights.scm` | GPL-3.0 |
| `cfml.scm` | `tree-sitter-cfml` 0.26.39 | `cfml/queries/highlights.scm` | MIT |
| `cfquery.scm` | `tree-sitter-cfml` 0.26.39 | `cfquery/queries/highlights.scm` | MIT |
| `cfscript.scm` | `tree-sitter-cfml` 0.26.39 | `cfscript/queries/highlights.scm` | MIT |
| `d.scm` | `tree-sitter-d` 0.8.2 | `queries/highlights.scm` | MIT |
| `dafny.scm` | `tree-sitter-dafny` 0.1.0 | `queries/highlights.scm` | MIT |
| `dm.scm` | `tree-sitter-dm` 0.25.4 | `queries/highlights.scm` | MIT |
| `fsharp-signature.scm` | `tree-sitter-fsharp` 0.3.12 | `fsharp_signature/queries/highlights.scm` | MIT |
| `gomod.scm` | `tree-sitter-gomod-orchard` 0.5.3 | `queries/highlights.scm` | MIT |
| `gwbasic.scm` | `tree-sitter-gwbasic` 0.2.0 | `queries/highlights.scm` | MIT |
| `ink.scm` | `tree-sitter-ink-lbz` 0.0.5 | `queries/highlights.scm` | MIT |
| `integerbasic.scm` | `tree-sitter-integerbasic` 3.0.0 | `queries/highlights.scm` | MIT |
| `julia.scm` | `tree-sitter-julia` 0.23.1 | `queries/highlights.scm` | MIT |
| `matlab.scm` | `tree-sitter-matlab` 1.3.1 | `queries/neovim/highlights.scm` | MIT |
| `merlin6502.scm` | `tree-sitter-merlin6502` 4.0.0 | `queries/highlights.scm` | MIT |
| `msbasic2.scm` | `tree-sitter-msbasic2` 0.2.0 | `queries/highlights.scm` | MIT |
| `newick.scm` | `tree-sitter-newick` 1.1.0 | `queries/highlights.scm` | see crate |
| `nginx.scm` | `tree-sitter-nginx` 1.0.1 | `queries/highlights.scm` | MIT |
| `pascal.scm` | `tree-sitter-pascal` 0.10.2 | `queries/highlights.scm` | MIT |
| `pgn.scm` | `tree-sitter-pgn` 1.4.3 | `queries/highlights.scm` | BSD-2-Clause |
| `plpgsql.scm` | `tree-sitter-postgres` 1.2.4 | `plpgsql/queries/highlights.scm` | BSD-3-Clause |
| `postgres.scm` | `tree-sitter-postgres` 1.2.4 | `postgres/queries/highlights.scm` | BSD-3-Clause |
| `prisma.scm` | `tree-sitter-prisma-io` 1.6.0 | `queries/highlights.scm` | MIT |
| `proto.scm` | `tree-sitter-proto` 0.6.0 | `queries/highlights.scm` | MIT |
| `qbasic.scm` | `tree-sitter-qbasic` 0.2.0 | `queries/highlights.scm` | MIT |
| `systemverilog.scm` | `tree-sitter-systemverilog` 0.4.1 | `queries/highlights.scm` | MIT |
| `templ.scm` | `tree-sitter-templ` 2.2.0 | `queries/templ/highlights.scm` | MIT |
| `vue.scm` | `tree-sitter-vue-next` 0.1.0 | `queries/html_tags/highlights.scm`, `queries/vue/highlights.scm` | MIT |
