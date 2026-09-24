# Git a release notes

ArchGen pracuje s Gitom na dvoch miestach:

1. **Git lišta** pod názvom projektu: stav priečinka projektu, **Commit…** a **Push**.
2. **Release notes…**: poznámky k vydaniu z commitov ľubovoľného Git repozitára podľa šablóny.

Všetko volá nainštalovaný `git` (ako doterajšia *Git history*); ArchGen nemá vlastnú
implementáciu Gitu ani vlastné prihlasovanie.

## Git lišta

Zobrazí sa, keď je projekt uložený v priečinku: `Git: main · ↑1 · 3 changes`.

- **Stav** sa číta po každom otvorení a uložení a tlačidlom **Refresh**. Zobrazuje vetvu,
  predstih/oneskorenie voči upstreamu a zmeny **iba v priečinku projektu** (nie v celom
  repozitári). Čítanie nemení repozitár ani index (`GIT_OPTIONAL_LOCKS=0`, bez fsmonitor).
- **Commit…** ukáže zoznam zmenených súborov a správu (predvolene `docs: update <projekt>`).
  - Commit obsahuje **iba priečinok projektu** (`git add --all -- .` a `git commit -- .`).
    Čo máš v repozitári stagnuté inde, zostane stagnuté a do commitu nepôjde.
  - Commit obsahuje **uložený** obsah. Pri neuložených úpravách je tlačidlo zablokované.
  - Ak sa priečinok medzitým zmenil mimo ArchGen (git pull, iný editor), commit sa odmietne
    rovnakým odtlačkom obsahu, aký chráni ukladanie.
  - Do commitu idú aj ďalšie súbory, ktoré v priečinku projektu nie sú ignorované (napr.
    `templates/release-notes/*.md`); zoznam v dialógu ich ukazuje pred potvrdením.
  - Git hooky repozitára (napr. `pre-commit`) sa spúšťajú ako pri commite v termináli.
- **Push** je aktívny, keď má vetva upstream a lokálne commity navyše. Po potvrdení pošle
  **iba aktuálnu vetvu do jej upstreamu** (`git push <remote> HEAD:<upstream>`), bez ohľadu na
  `push.default`, nikdy force push. Limit 180 s. Prihlásenie rieši Git Credential Manager alebo
  SSH agent; ArchGen sa na heslo nepýta (`GIT_TERMINAL_PROMPT=0`). Keď vetva upstream nemá,
  ArchGen povie, ako ho raz nastaviť (`git push -u origin <vetva>`).
- Pull/merge zámerne nie sú v aplikácii: menia súbory pod otvoreným projektom. Po pulle
  v inom nástroji projekt znova otvor (uloženie by sa inak odmietlo ako súbežná zmena).

## Release notes z Gitu

**Release notes…** otvorí dialóg. Nič sa nikam neposiela a repozitár sa nemení.

1. **Repository folder**: predvolene priečinok projektu; **Choose folder…** vyberie iný
   repozitár (napr. kód aplikácie, ku ktorej je dokumentácia).
2. **Rozsah**: *From* (vylúčené) a *To* (zahrnuté) – tag, vetva, hash alebo výraz ako `HEAD~20`.
   Predvolene od posledného tagu po `HEAD`; ak `HEAD` práve je posledný tag, od predposledného
   tagu po posledný (dialóg otvorený hneď po otagovaní ukáže to vydanie). Prázdne *From* =
   celá história. *Only folder* obmedzí commity na podpriečinok (monorepo). Merge commity sú
   predvolene vynechané.
3. **Šablóna**: vstavané, projektové (`templates/release-notes/*.md` v priečinku projektu)
   alebo **Load template file…**. **Edit template** umožní šablónu upraviť s okamžitým náhľadom.
4. Náhľad, počty a zoznam **Left out** (čo šablóna vynechala a prečo).
5. **Create document** (nový dokument so šablónou *release-notes*), **Insert at top of …**
   (nové vydanie navrch aktuálneho dokumentu, napr. priebežného changelogu), **Save as Markdown
   file…** alebo **Copy Markdown**. Vloženie je bežná úprava s Undo.

Keď sa po načítaní zmení repozitár alebo rozsah, akcie sa zablokujú, kým commity nenačítaš znova.
Poznámky tak nikdy nevzniknú z iného rozsahu, než aký vidíš.

Texty commitov sú technické. Po vložení označ sekcie a použi **AI Rework** („prepíš pre
zákazníkov, slovensky"), výsledok prejde bežným *Review changes*.

### Ako sa čítajú commity

Podľa [Conventional Commits](https://www.conventionalcommits.org/): `typ(scope)!: popis`.

| Commit | typ | scope | breaking |
|---|---|---|---|
| `feat(api): add refunds (#12)` | feat | api | nie |
| `fix!: new tax model` | fix | | áno (`!`) |
| telo s `BREAKING CHANGE: …` | podľa hlavičky | | áno, text z pätičky |
| `Update readme` | other | | nie |
| merge commit | merge | | nie |
| `Revert "feat: x"` | revert | | nie |

Issues sa hľadajú v hlavičke aj tele: `#123` a kľúče typu `PAY-17` (bez `UTF-8`, `ISO-9001`,
`SHA-256` a podobných). **Breaking change sa nikdy nevynechá**, ani keď je jeho typ vo `exclude`.

### Vstavané šablóny

| Šablóna | Na čo |
|---|---|
| Keep a Changelog | blok `## [verzia] - dátum` na začiatok `CHANGELOG.md`; Added / Fixed / Changed / Removed / Other |
| Technical release notes | pre tím: všetky zmeny s commitom, autorom, issues a zoznam prispievateľov |
| Poznámky k vydaniu pre zákazníkov (SK) | Novinky / Opravy / Vylepšenia výkonu, kroky pri aktualizácii; technické typy skryté |

Pri dokumente v slovenčine sa predvolene ponúkne slovenská šablóna.

### Vlastná šablóna

Markdown s hlavičkou (malá podmnožina YAML) a značkami v štýle Mustache:

```markdown
---
name: Tímové release notes
description: Krátky popis v zozname šablón
groups:
  - title: Novinky
    types: feat
  - title: Opravy
    types: [fix, hotfix]
  - title: Ostatné
    types: "*"
exclude: chore, ci, build, test, style, merge
issueUrl: https://jira.example.com/browse/{id}
commitUrl: https://git.example.com/repo/commit/{hash}
---
# Vydanie {{version}} ({{date}})
{{#hasBreaking}}

## Pozor pri aktualizácii
{{#breaking}}
- {{breakingNote}}
{{/breaking}}
{{/hasBreaking}}
{{#groups}}

## {{title}}
{{#items}}
- {{#scope}}**{{scope}}:** {{/scope}}{{description}}{{#issueList}} ({{issueList}}){{/issueList}}
{{/items}}
{{/groups}}
```

- **groups**: commit patrí do prvej skupiny, ktorej `types` obsahuje jeho typ; `"*"` berie
  zvyšok. Commit bez skupiny sa vynechá (a objaví sa v *Left out*).
- **exclude**: vynechané typy (okrem breaking changes).
- **issueUrl / commitUrl**: odkazy; `{id}` je číslo (`#12` → `12`) alebo kľúč (`PAY-17`),
  `{hash}` celý hash commitu. Pri issues sú odkazy v `{{#issues}}{{id}} {{url}}{{/issues}}`.
- Premenné: `version`, `date`, `repository`, `from`, `to`, `range`, `stats.commits`,
  `stats.hidden`, `stats.total`, `stats.contributors`, `truncated`, `hasBreaking`,
  `contributorList`; zoznamy `groups` (`title`, `count`, `items`), `commits`, `breaking`,
  `contributors` (`name`, `commits`).
- Položka (`items`, `commits`, `breaking`): `description`, `scope`, `type`, `subject`, `body`,
  `shortHash`, `hash`, `author`, `date`, `issues`, `issueList` (issues, ktoré ešte nie sú v
  popise), `commitUrl`, `breaking`, `breakingNote`.
- `{{#x}}…{{/x}}` zopakuje blok pre každý prvok zoznamu alebo ho vykreslí raz, ak je `x`
  neprázdne; `{{^x}}…{{/x}}` iba ak je `x` prázdne; `{{! komentár }}`. Riadok, na ktorom je
  iba sekčná značka, nezanechá prázdny riadok. Výstup je Markdown, bez HTML escapovania.
- Chyba v šablóne (neuzavretá sekcia, zlá hlavička) sa ukáže hneď pri úprave s číslom riadku.

**Save template to project…** uloží šablónu do `templates/release-notes/<názov>.md` v priečinku
projektu. Commitni ju a celý tím bude mať rovnaké release notes. Súbory v `templates/` nepatria
do manifestu projektu: uloženie projektu ich nikdy neprepíše ani nezmaže.

## Limity

- Najviac 2000 commitov na rozsah (novšie); dialóg to povie. Telo commitu najviac 16 KiB.
- Šablóna najviac 256 KiB, v projekte najviac 100 šablón.
- Autor sa preberá ako meno (s `.mailmap`), e-mailové adresy sa nečítajú.
- Git musí byť nainštalovaný a v `PATH`. Repozitár iného používateľa Git odmietne
  (`safe.directory`) a ArchGen to povie.

## Overenie

- Rust (`git.rs`, `release.rs`): reálny Git repozitár v dočasnom priečinku – tagy (anotované aj
  ľahké), rozsahy, filter podpriečinka aj z podpriečinka repozitára, merge commity, odmietnutie
  `--option` a `a..b` ako revízie, commit iba priečinka projektu pri inak stagnutom súbore,
  odmietnutie zastaraného odtlačku, push do lokálneho bare remote vrátane `push.default=matching`.
- TypeScript (`tests/release-notes.test.mjs`, `tests/documents.test.mjs`): parser commitov,
  hlavička šablóny, Mustache podmnožina, všetky vstavané šablóny, delenie na sekcie.
- UI (`tests/e2e/git-release.spec.mjs`, mockované IPC s kontrolou názvov argumentov): celý tok
  release notes, zastaraný rozsah, úprava a uloženie šablóny, commit s neuloženými zmenami,
  push s potvrdením, priečinok mimo Gitu.
- Natívny dialóg výberu priečinka, Git Credential Manager a push na skutočný server v desktopovej
  aplikácii neboli v tomto prírastku overené.
