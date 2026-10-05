# Edición del arreglo: referencia Ableton Live 12

Referencias oficiales consultadas:
- [Arrangement View](https://www.ableton.com/en/manual/arrangement-view/)
- [Clip View](https://www.ableton.com/en/manual/clip-view/)
- [Editing MIDI](https://www.ableton.com/en/live-manual/12/editing-midi/)

## Implementado en esta entrega

El menú Edit del arreglo y el menú contextual de cada clip comparten las mismas operaciones.
Los atajos actúan sobre clips cuando el piano roll no tiene el foco.

| Operación | Audio | MIDI |
| --- | --- | --- |
| Copiar, cortar, pegar en marcador | Sí | Sí |
| Duplicar después del clip | Sí | Sí, en pista vecina si la pista destino tiene región |
| Dividir en marcador | Sí, misma pista | Sí, segunda región en pista vecina |
| Mover, recortar inicio/final | Sí | Sí |
| Estirar ×2/×½ o Shift+arrastre del borde derecho | Resampling: cambia tono | Escala tiempo y duración de notas |
| Reversa | Nuevo WAV del segmento visible | Invierte posición temporal de notas |
| Recortar contenido a límites del clip | Nuevo WAV | Elimina contenido fuera de región |
| Normalizar y fades de 10 ms | Nuevo WAV | — |
| Transposición semitono/octava, cuantización 1/16, legato, activar/desactivar | — | Sí |
| Deshacer/rehacer | Sí | Sí |

Los WAV de las ediciones se guardan en media con nombre único. El archivo original permanece intacto.
El portapapeles de clips es interno a Velvet y se limpia al cambiar de proyecto.

## Diferencias que todavía impiden equivalencia completa

- El modelo actual tiene una sola región MIDI por pista. Clips MIDI independientes en la misma pista y edición simultánea de varias regiones requieren modificar el modelo, el editor y el motor.
- El motor de audio reproduce a velocidad variable; no tiene estiramiento con conservación de tono, Warp Markers ni los modos Warp de Live.
- Los fades implementados se imprimen en un nuevo WAV. Fades editables con curvas y crossfades entre clips requieren metadatos y DSP específicos.
- Todavía faltan selección múltiple de clips y rangos, cortar/copiar/pegar/duplicar/eliminar tiempo e insertar silencio entre pistas.
- Falta consolidación de varios clips y bounce de selecciones.
- Faltan envolventes y automatización por clip, gain/transposición independientes de velocidad y marcadores de loop por clip.
- No se implementaron comping/take lanes, edición de pistas enlazadas, extracción de groove, conversión audio a MIDI, separación de stems, MPE ni herramientas generativas MIDI de Live.

Esta entrega incorpora operaciones de edición útiles; no afirma paridad completa con Ableton.
