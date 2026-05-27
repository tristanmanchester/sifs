Inspect the indexed structure of one repository-relative file path. Use this
after search or list_files when you need to see the symbols, line spans,
breadcrumbs, and chunk boundaries in a file before reading raw chunks.

The tool only reports data from the selected SIFS index. It does not read
arbitrary unindexed files. Use `symbols_limit`, `chunks_limit`, and `no_chunks`
to keep large files bounded.
