# Third-party notices

## Theurgy

- Upstream: `klikli-dev/theurgy`, `release/v26.3-1.120.1`, commit `33b959c9ed0da1de2aff649e9728c1a03470b1bd`.
- Code and assets retain the upstream `LICENSES/` directory and `REUSE.toml` in the generated content addon.
- GeckoLib models, animations, book pages, translations and other assets retain their original attribution; CC-BY-4.0 assets are not relicensed.
- `data/theurgy/hermetica.json` derives from the Theurgy Hermetica book data (CC-BY-4.0, © klikli-dev); its page images are Theurgy's own `assets/theurgy/textures/gui/book/*.png`.

## Modonomicon 2.19.0

- `assets/theurgy/textures/gui/modonomicon/**` are the unmodified default theme sprites (`assets/modonomicon/textures/gui/sprites/modonomicon/themes/default/**`) of `com.klikli_dev:modonomicon-26.3-neoforge:2.19.0` (© klikli-dev), licensed **MIT AND CC-BY-4.0** (<https://github.com/klikli-dev/modonomicon#licensing>), copied so the native Hermetica screen looks like the Modonomicon book Theurgy ships with.
- `native/theurgy/book.rs` reimplements the Modonomicon GUI layout in Rust after Modonomicon's MIT-licensed client code; no Modonomicon code is linked.

## CodeDefinedGUI 1.14.0

- `assets/theurgy/textures/gui/codedefinedgui/**` are 20 unmodified GUI sprites of `com.klikli_dev:codedefinedgui-26.3-neoforge:1.14.0` (`klikli-dev/code-defined-gui`, tag `release/v26.3-1.14.0`; `assets/codedefinedgui/textures/gui/sprites/{core,filter,widget}/*.png`, © 2026 klikli-dev), the library Theurgy 26.3 bundles for its filter menus. Its `REUSE.toml` licenses `src/main/resources/**` and `src/generated/resources/**` **CC-BY-4.0** (its README names the assets' licence "CC-BY-SA-4.0" while linking the CC BY 4.0 text; the per-file REUSE annotation is followed here).
- The `codedefinedgui.filter.*` and `codedefinedgui.tooltip.show_extended` entries of `assets/theurgy/lang/en_us.json` are that release's generated English texts (same licence); the `ru_ru.json` entries are this port's translation of them.
- `src/filter_client.rs` reimplements the layout and behaviour of its `ListFilterScreen` / `AttributeFilterScreen` in Rust after CodeDefinedGUI's MIT-licensed code; no CodeDefinedGUI code is linked.

## NeoForge 26.3.0.76

- The generated addon includes functional JSON item/block tag definitions from the pinned `neoforge-26.3.0.76-universal.jar` so recipes using `#c:*` resolve at runtime.
- Pinned artifact metadata declares **LGPL 2.1** (`<name>LGPL 2.1</name>`), with the license URL recorded in the POM: <https://github.com/NeoForged/NeoForge/blob/1.13-pre/LICENSE.txt>.
- Source/project: <https://github.com/NeoForged/NeoForge>. The local POM is `.local/gradle/caches/modules-2/files-2.1/net.neoforged/neoforge/26.3.0.76/`.
- Only data tag JSON is redistributed; NeoForge code is not linked into the pure Rust engine or presented as original work. The build records the copied file count and the local deterministic build also performs its own content-policy check.

No provider credentials, player saves or launcher instance data are included in the addon or these notices.
