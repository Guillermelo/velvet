# Migración de Velvet a procesamiento continuo

## Objetivo

Velvet necesita procesamiento continuo de audio para reproducción, instrumentos,
preview de notas y efectos. Actualmente prepara un render del proyecto y después
reproduce ese audio. La migración reemplaza esa reproducción por un motor que
mantiene procesadores vivos y calcula bloques durante playback. La exportación
WAV mantiene un modo offline. La migración continua todavía no está implementada.

Proyecto: `C:\Users\Guillermo\Documents\DEV\Velvet`.
Entorno verificado: Windows, Rust, CPAL, eframe/egui y `vst3-host 0.9.0`.

## Contexto y trabajo que debe conservarse

Hay muchos cambios sin commit, incluidos archivos nuevos sin seguimiento.
Otro chat trabaja en piano roll y arreglo. El estado actual del directorio es la
base de trabajo; los cambios existentes deben preservarse. Los cambios propios
de la migración deben quedar identificables, sin resetear ni reemplazar el trabajo
ajeno. La migración no requiere reescribir el modelo del proyecto ni el piano roll.

La lógica pesada de audio ya está separada de la interfaz. Los botones llaman
funciones compartidas. La deuda concreta es que el código principal de la ventana
coordina preparación, trabajos pendientes, revisiones, reemplazo del mix y
sincronización de transporte. Esa coordinación necesita una interfaz de playback
clara que permita sustituir el motor sin repartir nueva lógica por los controles.

## Estado actual verificado

- M1, Omnisphere, Vital y Spire pasaron carga, MIDI, salida audible y finita,
  restauración de estado, editor nativo abierto/cerrado/reabierto, proyecto
  guardado/reabierto, osciloscopio y exportación WAV.
- La prueba audible alterna cambios grandes, compara el audio editado con el
  restaurado, comprueba archivos `.vstpreset`, instancias reutilizadas,
  instancias nuevas para proyecto/exportación y preview del piano roll.
- M1 también pasó selección de programas 0–3. Omnisphere pasó doce selecciones
  consecutivas de programa y cierre rápido de su instancia de editor.
- ValhallaVintageVerb pasó editor abierto/cerrado/reabierto, estado, procesamiento
  de efecto, osciloscopio, reapertura de proyecto y WAV. Los otros Valhalla y los
  demás efectos instalados todavía necesitan verificación.
- Última verificación registrada: 24 tests de audio aprobados y uno de hardware
  ignorado; 208 tests disponibles de la dependencia aprobados. Tres tests de la
  interfaz de plugins aprobados. El binario de la app compila.
- En la suite completa de app quedaron dos fallos gráficos de matriz/grilla:
  `hover_and_tempo_motion_preserve_the_grid_and_follow_transport` y
  `synced_clip_loop_and_waveform_share_the_musical_grid`; cuatro tests de hardware
  estaban ignorados. Este resultado pertenece al estado compartido observado y
  necesita actualizarse al iniciar el trabajo, sin revertir cambios ajenos.

## Correcciones VST3 existentes

Cargo usa una copia local del paquete publicado bajo `vendor/vst3-host`, mediante
`[patch.crates-io]`. Su licencia MIT y cambios están en `PATCHES.md`.
Los experimentos de `target/vst3-host-patch` no están conectados a Cargo.

1. Lectura de estados: las lecturas cortas con bytes devuelven éxito y su tamaño
   real; una lectura no vacía solicitada en EOF sin bytes devuelve fallo. M1 lee
   por bloques y antes podía quedarse con su sonido anterior o no terminar.
2. Cambios del editor: revisión monotónica sin consumir la cola del DSP,
   coalescencia del último valor por parámetro y descarte de valores GUI viejos
   antes de restaurar un estado.
3. Windows: OLE inicializado en el hilo dueño de editores; el guard tiene afinidad
   de hilo. Los módulos DLL se comparten por ruta canónica y permanecen cargados
   hasta terminar el proceso, con contexto de fábrica estable. Se evita ejecutar
   `ExitDll` por instancia; Omnisphere se colgaba allí al cerrar rápido.
   Los componentes/controladores sí reciben cierre y terminación normales.
4. Spire: negociación de tamaño después de adjuntar el editor, porque antes de
   adjuntarlo puede informar ancho cero.
5. Vital: reset MIDI después de restaurar el estado y procesado en un bloque
   anterior a las notas nuevas, para evitar voces anteriores. El render fresco
   y el reutilizado comparten preparación.
6. Todos los buses de salida de instrumentos reciben buffers; el rack usa la
   salida principal mono/estéreo. Los bloqueos por nombre de M1/Omnisphere se
   retiraron y no constituyen una solución aceptable.

En el sandbox restringido de comandos, M1 llegó a declarar 65 buses y fallar
dentro de M1.dll; Omnisphere también presentó fallos de editor. Las pruebas de
plugins necesitan un proceso normal de Windows, con recursos y configuración
del usuario. En Codex, estas ejecuciones requieren salir del sandbox mediante
el mecanismo de permisos disponible; esto no implica ejecutar Velvet como admin.

## Archivos y módulos relevantes

- `crates/audio/src/lib.rs`: mezcla offline, `LiveMixer`, `Player`, CPAL,
  transporte atómico, intercambio de mixes, scopes y tests.
- `crates/audio/src/plugins.rs`: carga, buses, captura de estado, preparación,
  scheduler MIDI offline y `RenderCache`.
- `crates/audio/src/devices.rs`: EQ, compresor, limitador y DSP interno.
- `crates/audio/src/synth.rs`: sintetizador interno, voces y envolventes offline.
- `crates/audio/src/beat.rs`: efecto temporal, delays, hold y envolventes.
- `crates/app/src/main.rs`: coordinación actual de reproducción, trabajos,
  revisiones, transporte, preparación y actualizaciones.
- `crates/app/src/plugins.rs`: ventanas nativas, captura debounced y estado.
- `crates/app/src/piano_roll.rs`: preview de notas; archivo compartido con otro chat.
- `vendor/vst3-host/src/realtime.rs`: runner existente que merece evaluación;
  su documentación reconoce mutexes internos y restricciones de afinidad.
  Su existencia no demuestra que resuelva un grafo completo o editores.
- `crates/audio/examples/plugin_smoke.rs`, `plugin_state_roundtrip.rs`,
  `plugin_profile.rs`: pruebas instaladas y mediciones reproducibles.
- `docs/vst3-host-architecture.md`, `docs/vst3-compatibility.md`:
  diagnóstico, límites y resultados anteriores.

## Alcance de la migración

### Motor y responsabilidad

Una interfaz de playback debe concentrar preparación del grafo, reproducción,
pausa, stop, seek, loops, cambios de proyecto, preview, errores y feedback.
La ventana comunica intención y muestra resultados; el motor administra el
ciclo de vida y la ejecución. La reproducción procesa bloques de duración
acotada, sin necesitar un render completo del arreglo para comenzar o editar.

Un grafo persistente debe conservar voces, fases, filtros, reducción de ganancia,
delays y colas de efectos entre bloques. Repetir la función offline sobre trozos
independientes reiniciaría esos estados y no cumple el objetivo.

El callback del dispositivo necesita un camino acotado: sin carga de DLL,
serialización de presets, decodificación, esperas del editor ni destrucción pesada.
El diseño debe justificar dónde se ejecuta DSP y cómo se entregan bloques a CPAL.
Un hilo de procesamiento con buffers acotados es procesamiento continuo si se
mantiene al día con la salida; no lo es un prerender del proyecto con otro nombre.
El uso de memoria para audio producido no debe crecer con la duración del arreglo.

### DSP, MIDI y transporte

- Instrumentos y efectos persistentes, con cambios aplicados en límites seguros.
- MIDI con offsets de muestra, note-off antes de retrigger cuando corresponda,
  canales, velocidad, notas silenciadas y liberación de voces correctas.
- Preview de notas sobre el motor continuo, sin preparar una nota entera antes
  de escucharla. La entrada MIDI externa de hardware es una extensión distinta
  si no está disponible hoy; su alcance debe quedar explícito.
- Play/pause/stop/seek/loops/tempo mantienen cursor y grid coherentes; no dejan
  notas colgadas ni reproducen trabajo de una revisión anterior.
- El EQ y el compresor actuales inicializan estado por llamada. El limitador
  usa lookahead offline sobre el buffer completo. Beat conserva delays/hold sólo
  dentro de una llamada. Necesitan procesadores persistentes, y el lookahead
  continuo necesita buffers acotados y tratamiento explícito de su latencia.
- Mixer, mute, solo, pan, master, monitor y metrónomo conservan su comportamiento.
- Compensación de latencia del plugin/grafo, inicio y final de notas y tails
  requieren pruebas. Un seek/loop necesita una política definida para resets,
  persecución de notas activas y colas de efectos.

### Plugins y editores

La afinidad de hilo de controladores, editores, OLE, configuración, estado y
teardown debe quedar explícita. Las llamadas lentas de estado o carga no deben
bloquear el callback de audio ni reiniciar todos los dispositivos por cada cambio.
Las instancias GUI/DSP separadas existentes pueden conservarse o reemplazarse
con una decisión justificada y comprobada; compartir un mutex con el callback
no resuelve las esperas. El reinicio de un plugin puede tener un intervalo local
de transición: el procesamiento continuo no hace instantáneas las cargas grandes.

Los scopes necesitan mostrar salida reciente real del dispositivo, con una
transferencia acotada que no reconstruya el audio completo del arreglo.
La exportación sigue siendo determinista donde el plugin lo permita y usa el
mismo comportamiento DSP/MIDI que playback, conducido en modo offline.

Las DLL siguen ejecutándose dentro del proceso. La migración no implica por sí
sola aislamiento frente a crashes de terceros; habilitar una feature de Cargo
no acredita aislamiento completo de audio, estado y editor.

## Verificación de efectos

Valhalla instalados: FutureVerb, Delay, FreqEcho, Plate, Room, Shimmer,
SpaceModulator, Supermassive, UberMod y VintageVerb. Están bajo las carpetas
`Valhalla DSP` y `ValhallaDSP` dentro de los VST3 comunes de Windows.
VintageVerb ya pasó; los demás requieren ejecución individual para identificar
qué plugin falla y en qué fase. Los otros efectos instalados también requieren
inventario y pruebas, con resultados diferenciados de los sintetizadores.

Ejemplo de prueba existente:

```powershell
cargo run -p velvet-audio --example plugin_smoke -- effect "C:\Program Files\Common Files\VST3\ValhallaDSP\ValhallaVintageVerb.vst3" --editor
```

## Criterios de aceptación

1. Playback comienza sin generar el audio completo del proyecto. La diferencia
   entre arreglos cortos y largos no genera una espera proporcional de render.
2. Notas nuevas y ediciones audibles usan bloques continuos. Las latencias se
   miden desde el evento hasta la salida, separadas de carga inicial/preset.
3. Procesar por bloques conserva continuidad de voces, filtros, compresor,
   limitador y delays. Tests comparan resultados contra referencias pertinentes.
4. Play, pausa, stop, saltos, loops, tempo, undo/redo y cambios rápidos no dejan
   voces colgadas ni audio obsoleto. Las transiciones tienen política documentada.
5. UI, scopes y editor permanecen utilizables durante reproducción y edición.
   Se registran underruns y sobrecarga; no se disimulan reproduciendo eternamente
   un mix viejo ni se presentan pruebas sin sonido como compatibilidad completa.
6. M1, Omnisphere, Vital y Spire conservan las pruebas anteriores y pasan playback
   continuo, presets repetidos y editores. Valhalla y efectos se verifican también.
7. WAV offline, apertura/guardado de proyecto, preview, instrumentos internos,
   effects/master/mixer y metrónomo siguen funcionando.
8. Los resultados incluyen pruebas automatizadas útiles, pruebas reales de CPAL
   cuando estén disponibles, mediciones de latencia/carga/underruns y limitaciones
   concretas. Compilar o escuchar un único preset no basta como aceptación.

La solución principal debe quedar conectada a la app, no sólo en un ejemplo o
prototipo. Los cambios de arquitectura deben quedar documentados y las pruebas
fallidas clasificadas sin mezclar problemas ajenos con la migración.

## Plataformas y formatos

La prioridad verificada es VST3 en Windows. VST2 no está implementado por esta
migración previa. Linux necesita pruebas propias de audio/editor: los binarios
Windows requieren una vía como Wine/yabridge, todavía no validada con Velvet.
La compatibilidad Windows demostrada no acredita automáticamente Linux.
