# ADR: prvý distribučný spike (Windows x64)

Stav 2026-09-06: lokálny PlantUML renderer aj izolovaný Python staging sú implementované. Skutočný release executable s vloženým frontendom prešiel PyO3 aj renderer smoke testom s DLL a knižnicami z prenosného priečinka, s minimálnym PATH a neplatným PYTHONHOME/PYTHONPATH. **Čistý Windows inštalátor a živá Ollama inferencia ešte nie sú overené.** PyO3 zostáva zachovaný; prechod na worker tento spike nevyžaduje.

## Lokálny renderer

V checkout-e spusti `./scripts/setup-renderer.ps1` cez PowerShell. Skript stiahne pevné oficiálne verzie, porovná SHA-256 pred rozbalením a odmietne prepísať existujúci renderer. Aktuálne približne 79 MB na stiahnutie a 181,4 MB rozbalených súborov:

- PlantUML 1.2026.8, SHA-256 `5e1ecfa8ecd32c90b03bbf3b1eb6f020943f98ab0fcf4032be31a0002ee2c462`.
- Eclipse Temurin JRE 21.0.12.1+1 Windows x64, SHA-256 archívu `d35f31e712f0fcf6ac5a093edc90204fbff22f720ba3950bd09d331d5e621636`.

Oficiálne zdroje: [PlantUML release](https://github.com/plantuml/plantuml/releases/tag/v1.2026.8), [Temurin release](https://github.com/adoptium/temurin21-binaries/releases/tag/jdk-21.0.12.1%2B1). JRE sa kopíruje aj s `legal/`; distribučná brána ešte vyžaduje súpis licencií a splnenie povinností všetkých dodávaných artefaktov vrátane PlantUML. Setup zámerne ponecháva staging v temp adresári a vypíše jeho cestu.

Binárky sú v ignorovanom `src-tauri/resources/renderer/`; Tauri konfigurácia ich zahŕňa medzi bundle resources. Na novom checkout-e treba setup vykonať pred buildom. Debug používa tento adresár, release používa Tauri resource directory. Žiadny implicitný fallback na systémovú Javu ani na sieťový renderer.

Pri diagrame klikni **Render locally (no upload)**. Zdroj ide cez stdin do privátnej Javy, SVG sa vracia cez stdout. Obsah dokumentu sa nemení. Verejný server zostáva samostatnou explicitnou voľbou a pri lokálnej chybe sa sám nespúšťa.

Ochrany: PlantUML [SANDBOX profil](https://plantuml.com/security), vyčistené prostredie procesu bez zdedených Java options a PATH, skryté okno, Smetana layout bez externého Graphviz, 256 KiB vstup, 4 MiB SVG, 64 KiB stderr, 256 MiB Java heap, 15 sekúnd a najviac dva paralelné procesy. SVG je sanitizované a vložené ako obrázok, nie aktívny DOM. Toto nie je OS kontajner ani dôkaz úplnej izolácie JVM; čistý Windows bezpečnostný test zostáva release bránou.

Prvá verzia podporuje jeden `@startuml` blok bez preprocessor/include direktív, s jednou výnimkou: C4 diagramy. Whitelist povoľuje presne týchto sedem C4 stdlib includes (porovnané po orezaní whitespace, nezávisle od veľkosti písmen, vždy vyslané rendereru v kanonickom tvare) — `!include <C4/C4>`, `<C4/C4_Context>`, `<C4/C4_Container>`, `<C4/C4_Component>`, `<C4/C4_Dynamic>`, `<C4/C4_Deployment>`, `<C4/C4_Sequence>` — a nič iné. Táto knižnica je súčasťou `plantuml.jar` (žiadny súborový ani sieťový prístup). Overené skutočným rendererom: C4 Context (Person/System/System_Ext/Rel/SHOW_LEGEND) aj C4 Container (System_Boundary/Container/Rel/LAYOUT_WITH_LEGEND). `!pragma layout smetana` sa vkladá pred include a funguje s ním bez problémov. Sekvenčné a class diagramy sú tiež overené skutočným rendererom. `!includeurl`, súborové/URL varianty `!include`, `!include <C4/../...>` (path traversal), `!import`, `!define`, `!pragma` mimo vloženého, `!theme`, iné stdlib knižnice a viacero `@start` blokov zostávajú zamietnuté ešte pred spustením Javy. Pri nepodporovanom zdroji je chyba, ktorá vymenúva povolené includes, a dostupný zdrojový kód, bez automatického odoslania.

## Python: dôkazy a zostávajúca brána

Odstránené osobné absolútne cesty z `.cargo/config.toml` a `src-tauri/build.rs`. Predvolený build použije `python` z PATH; pre dedikované prostredie nastav `PYO3_PYTHON` na jeho absolútnu cestu. PyO3 samo zistí import library. Build a Rust testy s touto konfiguráciou prešli.

`python -I scripts/probe-python-runtime.py` vypíše JSON inventár bez importu AI modulov a bez sieťových volaní. Nenahrádza uzamknutie závislostí ani smoke test importov. V aktuálnom prostredí: Python 3.14.0 x64 s `python314.dll`, DSPy 3.1.3, LangGraph 1.0.10, Pydantic 2.13.3, Ollama 0.6.1, LiteLLM 1.82.0, NumPy 2.4.0. Verzie sú pozorovanie vývojového prostredia, nie odporúčanie pre vydanie.

PyO3 sa dynamicky viaže na Python DLL, ktorú Windows potrebuje už pri štarte procesu. Samotné pridanie Python súborov do Tauri resources túto požiadavku nerieši. Oficiálny [PyO3 distribučný návod](https://pyo3.rs/v0.25.1/building-and-distribution.html) a [Python embeddable distribution](https://docs.python.org/3.14/using/windows.html#the-embeddable-package) sú základom ďalšieho overenia.

## Izolovaný Python staging a portable build

Build interpreter musí byť Windows x64 CPython 3.14 s pip. Používa sa len na prípravu balíka, nie na beh výslednej aplikácie. Systémové site-packages sa nemenia ani nekopírujú.

```powershell
python scripts/setup-python-runtime.py
npm run build
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml --offline
python scripts/stage-portable.py --executable src-tauri/target/release/arch-gen.exe
```

Predtým treba pripraviť aj JRE/PlantUML cez `scripts/setup-renderer.ps1`. `custom-protocol` je povinné: vloží frontend do executable; obyčajný Cargo build bez tejto feature používa vývojový režim Tauri. Packaging skript odmietne debug executable alebo executable bez vloženého frontendu.

- CPython 3.14.7 embeddable AMD64 z [oficiálneho archívu](https://www.python.org/ftp/python/3.14.7/), SHA-256 `d297e5ff019966817ad8502465176139f2d3d840fa4ed84b13bed399a6ab1f15`. Digest bol získaný z oficiálneho Sigstore bundle cez TLS; setup kontroluje SHA-256, **neoveruje podpisovú/release-identity reťaz Sigstore**.
- `requirements-win-x64.in` definuje východiskové verzie; `requirements-win-x64.lock` uzamyká všetkých 84 vybraných wheelov a SHA-256 pre CPython 3.14 Windows x64. Inštalácia používa `--require-hashes --only-binary=:all:`. Tento lock nie je univerzálny pre Linux, ARM ani iné Python verzie. NumPy 2.4.0 bol pri resolvovaní označený ako yanked pre compatibility bug; staging používa 2.4.6. Vývojové prostredie si ponechalo svoju pôvodnú verziu.
- Cache archívu a wheelov je `.runtime-cache/`. Inštalácia do nového stage beží bez indexu; `--offline` zakáže aj sťahovanie. Na preukázanie opakovateľnosti možno použiť `python scripts/setup-python-runtime.py --offline --destination .runtime-cache/python-offline-proof`. Existujúce cieľové adresáre sa nikdy neprepisujú.
- Runtime sa pripraví v `src-tauri/resources/python/`. `python314._pth` vedľa DLL určuje iba privátny stdlib, site-packages a engine, bez `import site`, používateľského site, registry import paths a vykonávania `.pth` súborov. Pri PyO3 inicializácii sa nevytvára bytecode v resources a `__main__.__file__` odkazuje na inertný `embedded_entry.py`, nie na náhodnú cestu z cwd.
- Portable priečinok obsahuje Python DLL vedľa `arch-gen.exe`; Windows loader tak nájde interpreter ešte pred Rust startupom. Engine sa hľadá explicitne pri executable; release nemá fallback na checkout alebo cwd. Renderer je v `resources/renderer/`.
- `stage-portable.py` vytvorí nový priečinok v `portable-builds/`, pribalí aktuálny engine, JRE a renderer a spustí `arch-gen.exe --offline-runtime-check` bez GUI. Kontroluje pôvod interpreter DLL aj native modulov, úplnosť závislostí vrátane extras, izolované import paths a lokálny renderer. Test má zakázané socket operácie a inference je **mockovaná**, nie živý model.
- `runtime-manifest.json` obsahuje verzie a licenčné metadáta balíkov; pôvodné `.dist-info`/licenčné súbory zostávajú pribalené. `portable-manifest.json` zaznamenáva smoke výsledok a SHA-256 jednotlivých súborov. Inventár nie je licencia/supply-chain audit: `licenseReviewComplete` a `cleanWindowsVerified` zostávajú false.

Nejde ešte o MSI/NSIS inštalátor. Neprenášaj iba `.exe`; treba celý portable priečinok. Generický Tauri bundle zatiaľ nepribaľuje Python na správne loader miesto a nesmie sa považovať za hotové vydanie. Windows WebView2 a systémové runtime požiadavky treba overiť na čistom stroji; Ollama a model zostávajú explicitnou externou požiadavkou iba pre AI funkcie.

Neúspešné stage priečinky sa ponechávajú na diagnostiku a skripty vypíšu ich cestu. Zdrojové zmeny aktualizuje nový portable build; zmena dependency locku vyžaduje zodpovedajúci nový Python staging. Packaging odmietne runtime so starším lockom.

Zostávajúce konkrétne kroky:

1. Čistý Windows smoke bez systémového Pythonu/Javy: štart UI, Open/Save, offline render, WebView2/VC runtime kontrola. Doterajší izolovaný proces bežal na vývojovom Windows, nie na čistom VM.
2. Živý Ollama smoke s deklarovaným modelom, timeoutmi a chybovým výstupom.
3. Licenčný a supply-chain audit, kontrola veľkosti a aktualizačného procesu; následne podpísaný MSI/NSIS inštalátor so správnym umiestnením Python DLL.

## Overenie

2026-09-06: 20 Rust testov (vrátane reálneho rendereru), 11 Python provider testov a 3 packaging testy prešli. Offline rebuild z cache aj release portable smoke prešli. Posledný testovací balík je `portable-builds/archgen-win-x64-he8r3esf/`, približne 391,5 MB (bez veľkosti výsledného manifestu). Smoke report potvrdzuje privátny `python314.dll`, validnú dependency closure, žiadne zachytené sieťové pokusy, `bundledFrontend: true`, `debugBuild: false` a `localRenderer: ok`. Modelová inferencia bola mockovaná. Manifest nie je digitálny podpis.

`cargo test --manifest-path src-tauri/Cargo.toml --offline` kontroluje validáciu vstupu a limit výstupu aj existujúce transakcie. Skutočný test privátneho rendereru vyžaduje setup a spúšťa sa explicitne:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --offline private_renderer_smoke -- --ignored
```

UI testy mockujú IPC: sanitizácia škodlivého SVG, zachovanie diagramového zdroja a chyba bez vzdialeného fallbacku. Nenahrádzajú natívny desktopový alebo inštalačný test.
