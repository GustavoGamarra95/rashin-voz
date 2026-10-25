# Rashin Voz

Habla con Rashin por voz desde la barra de Ryoku. Pulsas el micrófono, hablas,
vuelves a pulsar y Rashin te responde en pantalla y en voz alta. La conversación
es la misma del chat de Rashin (Super+S), con el mismo agente e historial.

## Piezas

- `engine/`: el motor, en Rust (`rashin-voz`). Graba el micrófono, transcribe,
  habla con Rashin y lee la respuesta.
- `service/`, `content/`: el plugin de Quickshell. Un icono de micrófono en la
  barra y un panel con el botón, el estado y la última respuesta.

## Instalación

```sh
cargo install --path engine --root ~/.local
ryoku plugin add ~/Documentos/ryoku-plugins/rashin-voz --bar --yes
```

Atajo opcional, en `~/.config/hypr/user.lua`:

```lua
hl.bind("SUPER + ALT + S", hl.dsp.exec_cmd("rashin-voz toggle"))
```

## Uso

- Di «Hola Rashin» y tu petición, o solo «Hola Rashin» y espera el «¿Sí?». Tras cada respuesta tienes unos segundos para seguir hablando sin repetirlo. Se desactiva en QS Bar Settings > Community.

- Clic en el micrófono de la barra: abre el panel.
- Botón grande: empieza a escuchar; otra pulsación envía lo dicho.
- Mientras habla, pulsar el botón la interrumpe.
- `rashin-voz toggle|listen|stop|cancel|silence|say <texto>` controla el motor
  en marcha desde un atajo o un script.
- Ajuste en QS Bar Settings > Community: leer o no las respuestas en voz alta.

## Qué ejecuta, lee y escribe

- Con «Hola Rashin» activado, el micrófono se escucha de forma continua y en local: solo los fragmentos con voz se transcriben con `voxtype` (modelo `base`) para buscar la frase; nada sale del equipo y los fragmentos se borran al instante.
- La voz natural es Chatterbox (`bin/rashin-voz-tts`, Python con PyTorch); si no está disponible, se usa piper.

- Ejecuta `rashin-voz serve` mientras el plugin está cargado. El motor lanza:
  - `pw-record` para grabar el micrófono (16 kHz, mono);
  - `voxtype transcribe` para pasar la voz a texto, en local;
  - `ryoku-rashin chat --follow` para enviar el mensaje a Rashin y recibir la
    respuesta;
  - `piper` (voz local) y `pw-play` para leer la respuesta.
- Lee la voz de piper de `~/.local/share/claude-tts/voices/es_MX-ald-medium.onnx`
  (otra con `RASHIN_VOZ_VOICE`; otro binario de piper con `RASHIN_VOZ_PIPER`).
- Escribe solo la grabación temporal `$XDG_RUNTIME_DIR/rashin-voz-<pid>.wav`
  (se borra al transcribir) y el socket `$XDG_RUNTIME_DIR/rashin-voz.sock`.
- No usa la red por su cuenta, no pide privilegios y no guarda secretos. Lo que
  Rashin haga con el mensaje depende de su agente configurado.

## Requisitos

`pipewire` (`pw-record`, `pw-play`), `voxtype`, `ryoku-rashin`, `piper` y Rust
para compilar el motor.

## Licencia

MIT. Ver [LICENSE](LICENSE).
