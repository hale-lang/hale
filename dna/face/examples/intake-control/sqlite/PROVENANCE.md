Minimal Pond SQLite wrapper from https://github.com/hale-lang/pond, commit `01b8643da1f63f14c27e7ca7ccc3c476b92c752f` (clean source). Apache-2.0; see LICENSE. driver.hl is deliberately excluded.

Local changes: Db create=false opens existing stores only; C/FFI add existing-only open, column type, autocommit and checked busy timeout. No SQLite engine is vendored.

Original file SHA-256 values:

```json
{
  "ffi.hl": "b12cdcdd6830af1a65b265ef048b24569845035b918b3c4530f5e48cffc3f3f7",
  "db.hl": "5ebc90675b1ab1ba4f843feefccbbc2ed17329a43e01bca446cb4f3767f9205a",
  "types.hl": "93b50c736b1eb931df6daa1148470559f9493af499067b43f889d9ca1de674c0",
  "glue.c": "ef0b86a15c43f3aaeefdc75b15a8c56d37b64a5597d36537e6fd16dcf5ff5ff2",
  "hale.toml": "ce87ba8c307f6cf306ca9e73d2188a646ce10c768a1fb002530929c61d074063"
}
```
