# Projection Mapping – native macOS-App

Projection Mapping für den MacBook-Beamer-Einsatz – als echte Mac-App in Rust, ohne Browser und ohne
Server. Gezeichnet wird über **Metal** (wgpu), die Bedienoberfläche ist mit egui gebaut.
Open Source (MIT).

Die Web-Version liegt weiter unter [HOAI-droid/Projection-Mapping](https://github.com/HOAI-droid/Projection-Mapping);
Projektdateien (JSON) lassen sich zwischen beiden Versionen austauschen.

## Funktionen

- **Zwei Fenster**: Steuerung auf dem Mac, Ausgabe auf dem Beamer (auf den Beamer ziehen, **F** = Vollbild –
  beim nächsten Start erscheint die Ausgabe dort automatisch wieder)
- **Flächen**: Viereck mit exaktem Corner Pin (Perspektive pro Pixel), Kreis, Dreieck und
  **Verzerrungsgitter** (bis 16×16, weich interpoliert) für gewölbte Objekte
- **Masken auf Masken** wie in MadMapper: Flächenmasken (hinzufügen / ausschneiden / Schnittmenge, mit weicher
  Kante) wandern mit der Fläche mit; **Masken-Ebenen** schneiden aus allen Ebenen darunter aus
- Freihand zeichnen oder Punkt für Punkt, **Snap** (standardmäßig an) an Ecken, Punkten, Kanten und Bildrand
- **Raster** über den Flächen mit einstellbarer Linienstärke, Testbild auf der Fläche, Kalibriergitter und Weißbild
- **41 Effekte** (Feuer, Lichter, Strobe, Disco, Laser, Kantenlauf, Wasser, Matrix, Musik-Visualisierungen …)
- **Bilder** (auch HEIC vom iPhone) und **Videos** (Hardware-Dekodierung), Einpassen/Füllen/Strecken
- **Live-Kamera** (MacBook-Kamera, iPhone über Continuity, USB-/HDMI-Grabber)
- **Audio-Reaktivität**: Mikrofon/Audio-Eingang (auch BlackHole für den Mac-Ton) oder eingebauter Player;
  Spektrum, Beat-Erkennung, Tempo, Tap; jeder Effekt-Regler und die Deckkraft lassen sich an die Musik koppeln
- **Milkdrop** – die originalen Winamp-Visualisierungen über **projectM** (~10 000 Presets per Knopfdruck)
- Mischmodi, Deckkraft, Blackout, Gesamthelligkeit, Rückgängig/Wiederholen, automatisches Sichern

## Installieren (fertige App)

1. Unter **[Releases → latest](https://github.com/HOAI-droid/Projection-Mapping-Native/releases/tag/latest)**
   `ProjectionMapping-macOS.zip` herunterladen und entpacken.
2. „Projection Mapping.app“ in den Ordner **Programme** ziehen.
3. Die App ist nicht von Apple notarisiert (dafür bräuchte es ein kostenpflichtiges Entwicklerkonto).
   Beim ersten Start daher einmal im Terminal:

   ```
   xattr -dr com.apple.quarantine "/Applications/Projection Mapping.app"
   ```

   (alternativ: Rechtsklick → Öffnen, bzw. Systemeinstellungen → Datenschutz & Sicherheit → „Trotzdem öffnen“).
4. Für **Videos, Kamera und den Musik-Player** wird ffmpeg gebraucht (einmalig):

   ```
   brew install ffmpeg
   ```

projectM (Milkdrop) ist in der App enthalten.

## Selbst bauen

Voraussetzungen: Xcode-Kommandozeilenwerkzeuge (`xcode-select --install`), Rust (<https://rustup.rs>),
`brew install cmake ffmpeg`.

```
git clone https://github.com/HOAI-droid/Projection-Mapping-Native.git
cd Projection-Mapping-Native
./scripts/build-macos.sh
open "dist/Projection Mapping.app"
```

Zum Entwickeln reicht `cargo run --release` (Milkdrop dann mit
`PM_PROJECTM_LIB=build/projectm/lib/libprojectM-4.dylib`, nachdem das Skript projectM einmal gebaut hat).

Tests: `cargo test` (inkl. GPU-Tests, die echte Szenen zeichnen und Pixel prüfen; alle Shader werden auch nach
Metal übersetzt). Die GitHub-Action baut die App auf einem Mac, testet sie dort mit Metal und legt sie unter
„Releases“ ab.

## Bedienung

| Taste | Wirkung |
|---|---|
| **E** | Bearbeiten ↔ Show (Umrisse/Griffe auf der Ausgabe ein/aus) |
| **F** | Ausgabe öffnen / Vollbild |
| **V / M** | Werkzeug Fläche / Maske |
| **S** | Snap ein/aus (⌥ beim Ziehen: ohne Snap) |
| **G / ⇧G** | Raster auf den Flächen / Kalibriergitter über das ganze Bild |
| **B** | Blackout |
| **T** | Tap-Tempo |
| Pfeile | Ecke/Fläche/Punkt 1 px (mit ⇧ 10 px) |
| Tab | nächste Ecke bzw. nächster Maskenpunkt |
| Entf / Enter / Esc | Punkt löschen / Maske fertig / Auswahl aufheben |
| ⌘Z / ⇧⌘Z | Rückgängig / Wiederholen |
| ⌘S / ⇧⌘S / ⌘O | Speichern / Speichern unter / Öffnen |
| Pinch / ⌥ + Scrollen | gewählte Fläche skalieren / drehen |

Bilder, Videos und Musik lassen sich einfach ins Fenster ziehen.

**Ton vom Mac** (Spotify, Apple Music, YouTube …) als Quelle: `brew install blackhole-2ch`, in der
Audio-MIDI-Konfiguration ein Multi-Ausgabegerät aus Lautsprechern + „BlackHole 2ch“ anlegen und es als Ausgabe
wählen; in der App unter Audio → Eingang „BlackHole 2ch“ wählen.

## Daten

Alles liegt in `~/Library/Application Support/Projection Mapping/`: der zuletzt bearbeitete Stand
(`autosave.json`), importierte Medien (`media/`), Milkdrop-Presets (`presets/`) und Einstellungen.

## Aufbau des Codes

| Datei | Inhalt |
|---|---|
| `src/model.rs` | Datenmodell (Projektformat 4, kompatibel zur Web-Version) |
| `src/geometry.rs` | Homographie, Dreieck, Verzerrungsgitter, Umrisse |
| `src/state.rs` | Bearbeitungszustand, Rückgängig, alle Bearbeitungsaktionen |
| `src/render/` | wgpu-Renderer (WGSL), Masken-Rasterung, Testbild |
| `src/effects.rs` + `assets/effects.json` | Effekt-Bibliothek (GLSL, über naga nach Metal) |
| `src/engine.rs` | verbindet alles: pro Bild Inhalte aktualisieren und Zeichenliste bauen |
| `src/media/` | Medienbibliothek, Video und Kamera (ffmpeg) |
| `src/audio/` | Audio-Eingang, Player, Analyse (FFT, Beat, BPM) |
| `src/milkdrop/` | projectM (zur Laufzeit geladen) mit eigenem OpenGL-Kontext |
| `src/ui/` | Steuerfenster, Ausgabefenster, Editor |

## Lizenzen

Eigener Code: MIT. projectM: LGPL-2.1 (als eigene Bibliothek mitgeliefert, Quelltext:
<https://github.com/projectM-visualizer/projectm>). Milkdrop-Presets: siehe Lizenz der jeweiligen Sammlung.
