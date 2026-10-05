# Piano roll de Velvet: referencia funcional y backlog

Investigación y primera entrega: 4 de octubre de 2026. Fuentes primarias: manual oficial de Image-Line consultado ese día. Es un inventario funcional, no una promesa de paridad ni una copia de su interfaz. Los estados describen el comportamiento disponible en Velvet; las familias con variantes faltantes están marcadas como parciales.

## Entrega 1: cobertura real y uso

Abrir un track MIDI seleccionándolo, con doble clic sobre su clip, con **Piano roll** en el rack o con **F7**. La ventana se puede cerrar y redimensionar. El selector de track permite cambiar de instrumento. El proyecto `examples/piano-roll-demo` permite probarlo con Dot sin medios externos; se genera con `cargo run -p velvet-app --example create_piano_demo -- examples/otro-demo`.

Implementado:

- Grid continuo de notas MIDI 0–127, teclado etiquetado, reglas de compases 4/4, minimap navegable, scroll horizontal/vertical, pan con botón medio, zoom en ambos ejes y fit de score/selección. Paleta, tipografías y notas translúcidas de Velvet.
- Draw con arrastre y longitud mediante Shift; Paint con relleno de pasos atravesados; Drum con mute/unmute de pasos; Erase con botón derecho o herramienta; Mute; Slice con corte diagonal/vertical; Select por rectángulo, tiempo y altura individual. Ctrl+Shift permite selección aditiva; selección por canal, mute, solapamientos e inversión.
- Movimiento múltiple, resize desde ambos extremos, stretch proporcional, clone con Shift+drag, clipboard interno entre tracks y duplicación a la derecha. Alt omite snap; flechas hacen nudge/transposición. Los bordes MIDI y tiempo cero se limitan en grupo para mantener los intervalos. Shift después de iniciar un movimiento bloquea pitch; Ctrl bloquea timing.
- Snap libre, valores rectos de 1/64 a un compás y tres resoluciones de tresillos. Duración heredada, duración/velocity/canal predeterminados. Catorce escalas con raíz, sombreado y snap de pitch. Trece stamps de nota/acorde, con inserción repetida o una sola vez.
- Inspector numérico de pitch, inicio, duración, velocity, canal y mute. Lane de velocity con dibujo, tails de duración y edición independiente de acordes mediante selección previa. Alt+wheel sobre notas cambia velocity. Altura configurable desde View.
- Mute persistente excluido del synth, render y eventos VST3. Colores corresponden a canales MIDI 1–16 enviados al VST3; Dot no usa timbres distintos por canal. Project/CLI comparten validación e historial.
- Quantize de inicios y longitudes con fuerza y swing; legato; staccato; chop por snap; glue contiguo por pitch/canal; inversión de tiempo y pitch; transposición; multiplicador y offset de velocity.
- Variantes básicas de arpegiado ascendente de un ciclo, strum ascendente con separación configurable, flam anterior con golpe al 65% de velocity, humanización de tiempo y velocity. Todas operan sobre selección o, si no hay selección, sobre el score completo. Son **parciales** frente a las herramientas de FL.
- Transporte del arreglo, seek desde regla, selección temporal y loop de ese rango; traducción correcta entre tiempo del source y del arreglo. Follow playhead. Preescucha corta y seca de Dot mientras está detenido; se sintetiza fuera del hilo de UI. VST3 reproduce el score mediante el renderer existente; preescucha VST3 y scrub quedan pendientes.
- Import MIDI SMF 0/1 con PPQ, canales, velocity, notas superpuestas y note-on con velocity cero como note-off. Mezcla todos los tracks del archivo y reemplaza el score en una acción undoable. Export de selección/score en formato 0 a 960 PPQ con tempo actual; omite mute. Rechaza formato 2, SMPTE y notas sin note-off. No importa mapas de tempo, CC, pedal, compases ni pitch bend. Archivos limitados a 16 MB / 10.000 notas.
- Guardar/cargar `.vscore` JSON con todas las propiedades de Velvet, escritura atómica y validación antes de editar. El clipboard interno no es el clipboard MIDI del sistema; `.vscore` no es FSC.
- Cada gesto actualiza el proyecto y la reproducción con el editor abierto, y se agrupa como un solo paso de undo al soltar. Escape restaura el score y la región originales sin agregar un paso de undo vacío; undo/redo y guardado conservan los datos musicales. Los cambios externos al score reconstruyen la selección para evitar índices obsoletos. Los recortes explícitos se preservan, y notas nuevas/retimadas pueden ampliar la región disponible.

Verificación: pruebas de puntero reales en egui para dibujo/movimiento/resize/clone/selección/corte/mute/velocity/cancelación; transformaciones y límites; loop con offsets; roundtrip MIDI; compatibilidad de proyectos anteriores; rechazo atómico de canal inválido; mute en synth y eventos VST3. Captura nativa en `examples/piano-roll-preview.png`. Preescucha audible requiere validación manual con el dispositivo del usuario; las pruebas no fuerzan salida audible.

Pendientes prioritarios para la siguiente parte: grupos persistentes; preescucha VST3, scrub y teclado de escritura; selección avanzada y ghost editable/filtrado; preview Apply/Cancel de transformaciones; arpegiador con dirección/octavas/gate; strum con velocity/tensión; progresiones deterministas; insertar/eliminar espacio y rotación. Más adelante: expresión por nota, MIDI live, automatización/CC/LFO, compases/marcadores y generadores/scripting.

Límite estructural actual: Velvet tiene una región MIDI por track. Múltiples clips MIDI independientes, patterns reutilizables y el flujo de Channel Rack/Playlist requieren ampliar el modelo; están pendientes. Los atributos pan, fine pitch, release velocity, Mod X/Y, slide y portamento no se muestran como controles funcionales hasta que el motor los reproduzca.

## Criterio de cobertura

- **Implementado**: la operación funciona, persiste donde corresponde y tiene verificación.
- **Parcial**: existe una variante útil; anotar qué falta frente a la referencia.
- **Pendiente**: no disponible todavía.
- **Dependencia de motor**: exige representación, reproducción o integración adicional.

Cada fila agrupa una familia. Implementar una parte no completa las demás opciones de esa fila. Las fases son una propuesta para Velvet, no una clasificación de Image-Line.

## 1. Edición y navegación

Fuente: [Piano roll](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll.htm).

| ID | Función de referencia | Fase | Estado |
| --- | --- | --- | --- |
| E01 | Dibujar; pintar; pintar batería; borrar; silenciar | 1 | Implementado |
| E02 | Mover; redimensionar ambos extremos; estirar selección | 1 | Implementado |
| E03 | Seleccionar por rectángulo, tiempo, altura; selección aditiva | 1 | Parcial: rectángulo/tiempo/pitch individual; falta rango de alturas |
| E04 | Lazo gestual; mantener pulsado para copiar/unir | 3 | Pendiente |
| E05 | Cortar notas; unir contiguas; bloquear tiempo/altura | 1 | Implementado |
| E06 | Duración heredada; duración predeterminada; edición sin snap | 1 | Implementado |
| E07 | Zoom horizontal/vertical; panorámica; zoom a selección | 1 | Implementado |
| E08 | Preescucha de teclado/notas; scrub; transporte | 1 | Parcial: transporte y preview Dot; falta VST3/scrub |
| E09 | Ghost notes internas/externas; filtros; edición directa | 2 | Parcial: ghost del arreglo con offsets; falta filtro/edición |
| E10 | Snap a escala: raíz, edición, entrada MIDI, sombreado | 1–3 | Parcial: edición/sombreado; falta entrada MIDI |
| E11 | Stamp: acordes, escalas, percusión, automático arriba/abajo | 1–3 | Parcial: 13 stamps; faltan escalas/percusión/automáticos |
| E12 | Propiedades: velocidad, pan, release, Mod X/Y, afinación | 1–3 | Parcial: velocity; falta expresión |
| E13 | Inspector numérico; edición de propiedades por rueda | 1–2 | Implementado |
| E14 | Lane inferior redimensionable; propiedades de acordes independientes | 1 | Parcial: altura desde View; falta splitter |
| E15 | Colores/canales; slide y portamento nativos | 3–4 | Parcial: colores/canales; falta slide/porta |
| E16 | Compases variables por patrón; waveform de referencia | 3 | Pendiente |

## 2. Menús, selección e intercambio

Fuente: [Piano roll Menu](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_menu.htm).

| ID | Función de referencia | Fase | Estado |
| --- | --- | --- | --- |
| M01 | Scores FSC; MIDI import/export; clipboard MIDI; partitura PDF | 3–4 | Parcial: SMF 0/1 y vscore; falta FSC/clipboard MIDI/PDF |
| M02 | Rotación; descartar duraciones; insertar/eliminar espacio; trim | 2 | Pendiente |
| M03 | Seleccionar aleatorias, color, offbeat, silenciadas, solapadas, apiladas | 2 | Parcial: color/mute/solapamientos |
| M04 | Selección temporal anterior/siguiente; ajustar a notas | 2 | Parcial: rango de selección; faltan anterior/siguiente |
| M05 | Agrupar/desagrupar; habilitar agrupamiento | 2 | Pendiente |
| M06 | Snap Main/Line/Cell/None; subdivisiones; eventos; marcadores | 1–3 | Parcial: None/divisiones/tresillos; falta Main/Line/Cell/eventos/marcadores |
| M07 | Grid: contraste, color, inversión, segmentos | 2 | Pendiente |
| M08 | Paletas; sombras; redondeado; etiquetas visibles | 3 | Pendiente |
| M09 | Minimap; indicador preciso; desplazamiento incremental | 2 | Parcial: minimap/playhead; falta scroll incremental |
| M10 | Escala automática/personalizada; longitud visible en lane | 3 | Parcial: duración en lane; falta escala auto/custom |
| M11 | Teclado: estilos, nombres personalizados, presets, etiquetas filtradas | 2–3 | Parcial: teclado flat etiquetado; faltan estilos/drum labels |
| M12 | Intercambiar/ocultar paneles; ventana separada | 3–4 | Parcial: ventana flotante interna; falta paneles/ventana SO |
| M13 | Marcadores individuales/periódicos; compás; tonalidad; longitud | 3 | Pendiente |
| M14 | Cambiar canal/control; localizar canal automáticamente | 2 | Parcial: selector de track; falta automatización/patterns |
| M15 | Preescucha durante playback; centrar playhead | 2 | Parcial: follow; falta audition durante playback |

## 3. Flujo de teclado

Fuente: [Keyboard & Mouse Shortcuts, Piano roll action](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/basics_shortcuts.htm).

| ID | Acción / referencia Windows | Fase | Estado |
| --- | --- | --- | --- |
| K01 | Herramientas P/B/C/D/E/T/Y/Z; Alt omite snap | 1 | Parcial: P/B/N/C/D/E/T/Z y Alt; falta Y |
| K02 | Copiar/cortar/pegar Ctrl+C/X/V; duplicar Ctrl+B | 1 | Implementado |
| K03 | Todo Ctrl+A; deseleccionar Ctrl+D; invertir Shift+I | 1 | Implementado |
| K04 | Transponer octava Ctrl+↑/↓; mover Shift+flechas | 1 | Implementado: flechas simples también editan |
| K05 | Nudge fino Alt+flechas; copiar arrastrando Shift | 1 | Implementado |
| K06 | Legato Ctrl+L; chop Ctrl+U; glue Ctrl+G | 1 | Implementado |
| K07 | Inicio cuantizado Shift+Q; cuantizador Alt+Q | 1–2 | Parcial: Shift+Q/Ctrl+Q y parámetros en Tools |
| K08 | Zoom PageUp/Down; presets Shift+1…5 | 2 | Pendiente |
| K09 | Propiedades doble clic; lane Shift+F; rueda Alt | 1–2 | Parcial: inspector/lane/rueda; falta ciclo de targets |
| K10 | Historial Ctrl+Z / Ctrl+Alt+Z; mute Alt+M | 1 | Parcial: Ctrl+Z/Ctrl+Shift+Z; mute por menú/herramienta |
| K11 | Cambio de canal G/H/J/K; teclado M | 2 | Parcial: selector de track y F7 |
| K12 | Contexto junto al cursor; menú y comandos accesibles | 2 | Pendiente |

Propuesta Velvet: mantener compatibilidad en acciones frecuentes, documentar divergencias y respetar foco de campos de texto. Una tecla no debe editar música mientras el usuario escribe nombres o valores. Undo debe tratar un arrastre entero como una operación. Selección y clipboard se deben mantener por IDs estables o reconstruirse tras una operación.

## 4. Transformaciones musicales

### Cuantización

Fuente: [Quantizer](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_qnt.htm).

- **Q01 — Implementado, fase 1:** cuantización rápida de inicios.
- **Q02 — Parcial: fuerza para inicio/duración; falta sensibilidad, fase 2:** fuerza gradual para inicio y duración/final; sensibilidad.
- **Q03 — Pendiente, fase 2:** modos cuantizar duración/final, conservar duración/final.
- **Q04 — Pendiente, fase 3:** grooves de score, mezcla de propiedades; rejilla de preview y templates personalizados.

Propuesta: no convertir siempre a ticks enteros al editar; conservar precisión suficiente para humanización. Separar resolución de almacenamiento y snap visual. Una operación sin selección debe indicar si afecta todo el score.

### Articulación

Fuente: [Articulate](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_articulate.htm).

- **A01 — Implementado, fase 1:** legato y staccato.
- **A02 — Pendiente, fase 2:** portato, huecos pequeños, cortar acordes superpuestos.
- **A03 — Pendiente, fase 2:** multiplicador de duración, variación y semilla.
- **A04 — Pendiente, fase 3:** usar longitudes originales; considerar o ignorar notas fuera de la selección.

### Chop y arpegios

Fuentes: [Chopper](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_chp.htm), [Arpeggiator](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_arpeggiate.htm).

- **C01 — Implementado, fase 1:** subdividir notas según snap.
- **C02 — Pendiente, fase 3:** chop con templates, multiplicador temporal, mezcla de propiedades, referencia absoluta/relativa y agrupamiento.
- **AR01 — Parcial: ascendente, un ciclo; faltan direcciones/octavas/gate, fase 2:** arpegios arriba/abajo/alternados, rango de octavas y gate.
- **AR02 — Pendiente, fase 3:** patrones personalizados, inversión, multiplicador temporal, sincronización Time/Block/Chord, mezcla de propiedades y agrupamiento.
- **AR03 — Pendiente, fase 4:** semántica de templates FSC por colores para sostener notas, eximir rango y definir longitud.

Propuesta: una variante de arpegiador que ordena las notas de un acorde es útil, pero no equivale al motor de templates de FL. Registrar como parcial hasta resolver patrones arbitrarios, reinicios y notas sostenidas.

### Strum y flam

Fuentes: [Strum](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_strum.htm), [Flam](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_flam.htm).

- **S01 — Parcial: timing; falta velocity, fase 2:** desplazar inicios de acordes por altura y variar velocidad.
- **S02 — Pendiente, fase 3:** curvas/tensión de tiempo y velocidad; finales independientes; conservar final; trigger anticipado; alternar dirección; cortar acordes.
- **F01 — Parcial: separación configurable; velocity fija al 65%, fase 2:** golpe adicional con separación y velocidad configurables.
- **F02 — Pendiente, fase 3:** tiempo absoluto o ligado al tempo, golpe anterior, presets y agrupamiento.

### Claw, limit y flip

Fuentes: [Claw Machine](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_claw.htm), [Limit](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_limit.htm), [Flip](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_flip.htm).

- **CL01 — Pendiente, fase 3:** período, eliminación periódica, distorsión temporal, descarte de notas cortas, estiramiento compensatorio.
- **LI01 — Pendiente, fase 2:** limitar rango y ajustar tonalidad/escala.
- **LI02 — Pendiente, fase 3:** ajuste arriba/abajo/alternado, wrap de octava y offset.
- **FL01 — Implementado, fase 1:** invertir altura y tiempo.
- **FL02 — Pendiente, fase 2:** inversión temporal conservando inicios como opción.

### Randomize y escala de niveles

Fuentes: [Randomizer](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_random.htm), [Scale Levels](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_scale.htm).

- **R01 — Parcial: jitter temporal/velocity; semilla interna sin UI, fase 2:** humanizar propiedades existentes con intensidad y semilla.
- **R02 — Pendiente, fase 3:** generación por raíz, escala, octava/rango, duración/variación, densidad y polifonía.
- **R03 — Pendiente, fase 3:** fusionar notas iguales, portamento aleatorio, reset previo, bipolaridad, seeds independientes de notas/propiedades.
- **SL01 — Implementado, fase 2:** multiplicador y offset de velocidad.
- **SL02 — Pendiente, fase 3:** centro y curva logarítmica/tensión.

Propuesta: semillas reproducibles, preview reversible y límites MIDI estrictos. Humanizar solo velocity no completa Randomizer; jitter temporal sería una extensión Velvet, y debe identificarse como tal.

## 5. Composición asistida

### Chord Progression Tool

Fuente: [Chord Progression Tool](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_chordprogression.htm).

- **CP01 — Pendiente, fase 2:** progresiones predefinidas, tonalidad, cantidad, octava, duración; acordes y bajo separados.
- **CP02 — Pendiente, fase 3:** nombres absolutos/romanos, entrada textual, inversiones, voicings, extensiones, bajo fijo, copy/paste, swap/slide, acordes de paso.
- **CP03 — Pendiente, fase 4:** generación contextual, análisis de melodía, alternativas y locks; convencional/aventurero; semillas, temperatura, contexto, no diatónicas y sesgo de loop.
- **CP04 — Pendiente, fase 3:** políticas para notas existentes: reemplazar, conservar, ajustar, vincular/desvincular.
- **CP05 — Pendiente, fase 4:** performance Arp/Chop/Humanize/Bassline; densidad, repeticiones, on-beat, morphing, phrasing; presets propios y aprendizaje desde MIDI.
- **CP06 — Pendiente, fase 3:** preview/solo, historial interno, guardar progresiones y ritmos.

Propuesta: empezar por teoría determinista y presets. El manual describe un modelo generativo propio; una sucesión I–V–vi–IV no equivale a ese modelo. Evitar prometer su motor, sus resultados o sus recursos.

### Riff Machine

Fuente: [Riff Machine](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_riff.htm).

- **RM01 — Pendiente, fase 4:** pipeline activable de ocho etapas: progresión de notas, acordes, arpegio, espejado, niveles/pan, articulación, groove y ajuste de rango/tonalidad.
- **RM02 — Pendiente, fase 4:** procesar score existente o generar; longitud en compases, presets de etapas, random/reset individual, semilla global, preview hasta etapa, aceptar.

Propuesta: reutilizar transformaciones ya verificadas para construir esta herramienta; no mantener ocho algoritmos duplicados. La dependencia práctica es completar primero las fases 2 y 3.

## 6. Automatización, MIDI y scripting

### Eventos y LFO

Fuentes: [Event Editor](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/automation_eventeditor.htm), [LFO](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_lfo.htm).

- **EV01 — Pendiente, fase 3:** eventos de control separados de propiedades de nota; targets por patrón/control; captura en vivo y dibujo.
- **EV02 — Pendiente, fase 3:** dibujo libre/rectas, borrado, selección temporal; interpolación lineal/spline y smoothing.
- **EV03 — Pendiente, fase 4:** inicializar valor actual; convertir eventos a clips de automatización; import/export de automatización.
- **LF01 — Pendiente, fase 3:** generar seno, triángulo o cuadrada sobre rango temporal; fase, nivel, amplitud y velocidad.
- **LF02 — Pendiente, fase 3:** interpolar parámetros iniciales/finales, sincronizar tempo y preview.

Una lane de velocity contiene valores ligados a notas; una curva CC contiene eventos independientes en el tiempo. No afirmar soporte LFO/eventos por dibujar barras de velocity.

### Importación

Fuente: [MIDI Data Import Dialog](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_midi.htm).

- **MI01 — Parcial: mezcla de tracks y reemplazo undoable; falta diálogo selectivo, fase 3:** selección de tracks/canales, mezclar o reemplazar, realinear inicio.
- **MI02 — Parcial: velocity cero interpretada como note-off; falta compás, fase 3:** importar cambios de compás; opción explícita para velocity cero.

Propuesta: resolver también tempos, PPQ, notas pendientes y archivos inválidos al diseñar la interoperabilidad. Esas son condiciones de robustez de Velvet, no una afirmación de paridad con el diálogo de FL.

### Scripts

Fuente: [Piano roll Scripting API](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_scripting_api.htm).

- **SC01 — Pendiente, fase 4:** scripts Python para añadir/modificar/eliminar notas y marcadores; selección temporal y PPQ.
- **SC02 — Pendiente, fase 4:** propiedades completas de nota, grupos, flags, canal/color y repeticiones.
- **SC03 — Pendiente, fase 4:** formularios de parámetros, menús por categorías, scripts de usuario y ejecutar último.

Propuesta: antes de añadir un runtime, definir cómo las operaciones de scripts pasan por la autoridad del proyecto, historial y validación. No hay evidencia de que Velvet tenga hoy un runtime compatible con `flpianoroll`; agregar un menú no produce esa compatibilidad.

### Captura de interpretación

Fuente: [Note / MIDI Recording](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/recording_scores.htm).

- **REC01 — Pendiente, fase 3:** capturar controlador MIDI o teclado de escritura en el score del instrumento activo.
- **REC02 — Pendiente, fase 3:** armar, cuenta previa, metronomo, filtro de notas/automatización y compensación temporal.
- **REC03 — Pendiente, fase 3:** cuantización de entrada: inicios, finales o conservar duración; elegir grid compatible.
- **REC04 — Pendiente, fase 4:** buffer retrospectivo de interpretación, recuperar score sin grabación armada.
- **REC05 — Pendiente, fase 3:** cancelar toma o deshacer sesión completa; incorporación al arreglo.

La captura pertenece al flujo de composición aunque su implementación cruce transporte, dispositivos y editor.

## 7. Restricciones propias del motor y formatos

Referencia técnica de propiedades: [Scripting API](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll_scripting_api.htm). Las restricciones slide/portamento constan en [Piano roll, Understanding Slides & Portamento](https://www.image-line.com/fl-studio-learning/fl-studio-online-manual/html/pianoroll.htm#Slide).

Observación local al iniciar la investigación: `crates/core/src/lib.rs` representaba `MidiNote` con `key`, `velocity`, `start_beats` y `length_beats`, y cada `Track` con una lista de notas y una región MIDI. Ese estado puede cambiar durante esta implementación.

Las siguientes son dependencias de implementación inferidas para Velvet:

| Necesidad | Trabajo previo propuesto |
| --- | --- |
| Mute persistente | Implementado: campo, validación y exclusión en renderer |
| Canal/color y grupos | Canal/color implementados y enviados al VST3; grupos pendientes |
| Release, pan, Mod X/Y, afinación | Persistencia más contrato del motor y plugin; no solo controles visuales |
| Slide/portamento | Motor nativo explícito; traducción para plugins según capacidades |
| Automatización/CC/pitch bend | Eventos temporales y programación en el motor |
| Ghost entre regiones | Referencias musicales con offsets de arreglo correctos |
| Compases/tonalidades | Marcadores/modelo musical; grid y export interoperables |
| FSC, palettes NCP, scripts PYSCRIPT | Formatos o APIs específicos; valorar alternativas propias |
| MIDI grabado | Input, timestamp, latency, transporte, overdub y cuantización |

No equiparar pan por nota con pan de track, release velocity con duración de release, ni color visual con canal MIDI. En FL, slide/portamento de piano roll están limitados a sus instrumentos nativos; no asumir soporte VST por dibujar el flag.

## 8. Orden sugerido para las próximas entregas

1. **Fase 1 — Componer y editar:** grid/teclado con estilo Velvet, crear/mover/resize/pintar/borrar, selección, velocity, snap, zoom, clipboard, transposición, historial y transformaciones básicas. Integración real con proyecto y render.
2. **Fase 2 — Trabajo cotidiano completo:** editor preciso, selección inteligente, grupos, ghost, menú/contexto, atajos, articulación, arpegios, strum, humanización, progresiones deterministas y cuantización gradual.
3. **Fase 3 — Profundidad musical:** templates/grooves, import/export MIDI, compases/keys, paneles de expresión, automatización/CC/LFO, drum labels, waveform y detalles faltantes de las herramientas.
4. **Fase 4 — Sistemas mayores:** Riff Machine, modelo de progresiones contextual, scripting, compatibilidad específica FL, partitura y múltiples ventanas.

Todas las funciones no marcadas como implementadas permanecen en este backlog. Mantener una sección de cobertura real después de cada entrega con IDs, limitaciones y verificación para que la siguiente parte tenga alcance concreto.
