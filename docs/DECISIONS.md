# Decisions

## 2026-08-14 - LP-Matching family-scoped, Bundled-DB v7
- Entscheidung: Jeder LP↔EN-Match (Shortcode, thumbnail_key, dosbox_conf, has_thumbnail) läuft über `same_group`/`family_expr`; `refresh_catalog` ruft die Verknüpfung nach dem Row-Copy selbst auf.
- Verworfen: thumbnail_key-Kopie in setup.rs behalten - zweite Kopie der Regel driftete bereits (unscoped), gelöscht zugunsten `propagate_lp_thumbnail_keys`.
- Grund: Titel UND Shortcodes wiederholen sich über Pack-Familien (GK2-DE bekam den eXoWin9x-Key, Cover 404te); der Katalog-Refresh stellte die kaputten Keys aus der Bundled-DB bei jedem Start wieder her.
- Gotcha: Die Bundled-DB stempelt IHRE eigene Version; solange sie älter ist als CATALOG_VERSION, läuft der Refresh bei jedem Start - Heilungen müssen also im Refresh selbst passieren, nicht nur in migrate().
