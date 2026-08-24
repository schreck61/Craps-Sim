# Font Notice

The fonts in this directory are Modified Versions (under the SIL Open Font
License 1.1) of third-party fonts, subsetted and renamed for embedding in this
application. They are regenerated deterministically by `tools/subset-fonts.sh`;
do not edit them by hand.

## Shipped files

| Shipped file | Original font (exact name, version) | Original copyright | Modifications |
|---|---|---|---|
| `LongrunDisplay-Medium.ttf` | Fraunces (variable), Version 1.000;[0bf87f6ff], release 1.000, instanced at wght=560 opsz=72 SOFT=0 WONK=1 | Copyright 2020 The Fraunces Project Authors (github.com/undercasetype/Fraunces) | Instanced from the variable font (no static 72pt Medium exists upstream); subsetted to the Longrun glyph set; renamed per OFL requirements for Modified Versions |
| `LongrunSans-Regular.ttf` | Inter Regular, Version 4.001;git-9221beed3, release v4.1 | Copyright 2016 The Inter Project Authors (https://github.com/rsms/inter) | Subsetted to the Longrun glyph set; renamed per OFL requirements for Modified Versions |
| `LongrunSans-Medium.ttf` | Inter Medium, Version 4.001;git-9221beed3, release v4.1 | Copyright 2016 The Inter Project Authors (https://github.com/rsms/inter) | Subsetted to the Longrun glyph set; renamed per OFL requirements for Modified Versions |
| `LongrunSans-SemiBold.ttf` | Inter SemiBold, Version 4.001;git-9221beed3, release v4.1 | Copyright 2016 The Inter Project Authors (https://github.com/rsms/inter) | Subsetted to the Longrun glyph set; renamed per OFL requirements for Modified Versions |
| `LongrunMono-Regular.ttf` | IBM Plex Mono Regular, Version 2.3 | Copyright 2017 IBM Corp. with Reserved Font Name "Plex" | Subsetted to the Longrun glyph set; renamed per OFL Reserved Font Name requirements. IBM Plex Mono's default zero is already dotted (distinguishable from O), so no feature freezing was needed |
| `LongrunMono-Medium.ttf` | IBM Plex Mono Medium, Version 2.3 | Copyright 2017 IBM Corp. with Reserved Font Name "Plex" | Subsetted to the Longrun glyph set; renamed per OFL Reserved Font Name requirements. IBM Plex Mono's default zero is already dotted (distinguishable from O), so no feature freezing was needed |

## Internal font names

All naming name-table entries (nameIDs 1, 3, 4, 6, 16, 17) were rewritten to
the Longrun names; WWS aliases (nameIDs 21, 22) were removed. The original
copyright notice (nameID 0) and the OFL license text and URL (nameIDs 13, 14)
are retained inside every font, as the OFL requires.

| Shipped file | Family (nameID 1) | Full name (nameID 4) | PostScript name (nameID 6) |
|---|---|---|---|
| `LongrunDisplay-Medium.ttf` | Longrun Display | Longrun Display Medium | LongrunDisplay-Medium |
| `LongrunSans-Regular.ttf` | Longrun Sans | Longrun Sans Regular | LongrunSans-Regular |
| `LongrunSans-Medium.ttf` | Longrun Sans Medium | Longrun Sans Medium | LongrunSans-Medium |
| `LongrunSans-SemiBold.ttf` | Longrun Sans SemiBold | Longrun Sans SemiBold | LongrunSans-SemiBold |
| `LongrunMono-Regular.ttf` | Longrun Mono | Longrun Mono Regular | LongrunMono-Regular |
| `LongrunMono-Medium.ttf` | Longrun Mono Medium | Longrun Mono Medium | LongrunMono-Medium |

## Renaming rationale

Subsetting produces OFL "Modified Versions". IBM Plex declares the Reserved
Font Name "Plex" (see its copyright line), so its Modified Versions must not
be named with it. Inter declares no Reserved Font Name, and the Fraunces 1.000
release's OFL text declares none either, but all six fonts were renamed to the
Longrun families for consistency and to avoid presenting subset fonts under
their original names.

## Sources

- Inter: <https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip>
  (static TTFs from `extras/ttf/`)
- Fraunces: <https://github.com/undercasetype/Fraunces/releases/download/1.000/UnderCaseType_Fraunces_1.000.zip>
  (variable TTF from `Fonts - Desktop/`)
- IBM Plex Mono: <https://github.com/google/fonts/tree/main/ofl/ibmplexmono>
  (static TTFs, pinned to commit `0b58fb370093f9a9f4ff785d94405710b79de67c`)

## Licenses

Each family's complete SIL Open Font License 1.1 text, with its original
copyright line, is in this directory:

- `LICENSE-Fraunces.txt`
- `LICENSE-Inter.txt`
- `LICENSE-IBMPlexMono.txt`
