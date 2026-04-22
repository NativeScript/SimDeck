type StreamCanvas = HTMLCanvasElement | OffscreenCanvas;

const VERTEX_SHADER_SOURCE = `#version 300 es
layout(location = 0) in vec2 a_position;
layout(location = 1) in vec2 a_texCoord;

out vec2 v_texCoord;

void main() {
  v_texCoord = a_texCoord;
  gl_Position = vec4(a_position, 0.0, 1.0);
}
`;

const FRAGMENT_SHADER_SOURCE = `#version 300 es
precision mediump float;

in vec2 v_texCoord;
uniform sampler2D u_texture;

out vec4 outColor;

void main() {
  outColor = texture(u_texture, v_texCoord);
}
`;

function compileShader(
  gl: WebGL2RenderingContext,
  type: number,
  source: string
): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) {
    throw new Error("Unable to allocate a WebGL shader.");
  }

  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    return shader;
  }

  const infoLog = gl.getShaderInfoLog(shader) ?? "Unknown shader compile failure.";
  gl.deleteShader(shader);
  throw new Error(infoLog);
}

function createProgram(gl: WebGL2RenderingContext): WebGLProgram {
  const vertexShader = compileShader(gl, gl.VERTEX_SHADER, VERTEX_SHADER_SOURCE);
  const fragmentShader = compileShader(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER_SOURCE);
  const program = gl.createProgram();
  if (!program) {
    gl.deleteShader(vertexShader);
    gl.deleteShader(fragmentShader);
    throw new Error("Unable to allocate a WebGL program.");
  }

  gl.attachShader(program, vertexShader);
  gl.attachShader(program, fragmentShader);
  gl.linkProgram(program);
  gl.deleteShader(vertexShader);
  gl.deleteShader(fragmentShader);

  if (gl.getProgramParameter(program, gl.LINK_STATUS)) {
    return program;
  }

  const infoLog = gl.getProgramInfoLog(program) ?? "Unknown program link failure.";
  gl.deleteProgram(program);
  throw new Error(infoLog);
}

function getWebGL2Context(canvas: StreamCanvas): WebGL2RenderingContext | null {
  return canvas.getContext("webgl2", {
    alpha: false,
    antialias: false,
    depth: false,
    desynchronized: true,
    powerPreference: "high-performance",
    premultipliedAlpha: false,
    preserveDrawingBuffer: false,
    stencil: false
  } as WebGLContextAttributes & { desynchronized: boolean }) as WebGL2RenderingContext | null;
}

export class VideoFrameRenderer {
  private readonly canvas: StreamCanvas;
  private readonly gl: WebGL2RenderingContext;
  private readonly program: WebGLProgram;
  private readonly texture: WebGLTexture;
  private readonly vertexArray: WebGLVertexArrayObject;
  private readonly vertexBuffer: WebGLBuffer;

  constructor(canvas: StreamCanvas) {
    this.canvas = canvas;

    const gl = getWebGL2Context(canvas);
    if (!gl) {
      throw new Error("This browser does not support WebGL2.");
    }

    this.gl = gl;
    this.program = createProgram(gl);

    const vertexArray = gl.createVertexArray();
    if (!vertexArray) {
      throw new Error("Unable to allocate a WebGL vertex array.");
    }
    this.vertexArray = vertexArray;

    const vertexBuffer = gl.createBuffer();
    if (!vertexBuffer) {
      throw new Error("Unable to allocate a WebGL vertex buffer.");
    }
    this.vertexBuffer = vertexBuffer;

    const texture = gl.createTexture();
    if (!texture) {
      throw new Error("Unable to allocate a WebGL texture.");
    }
    this.texture = texture;

    gl.bindVertexArray(vertexArray);
    gl.bindBuffer(gl.ARRAY_BUFFER, vertexBuffer);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([
        -1, -1, 0, 1,
        1, -1, 1, 1,
        -1, 1, 0, 0,
        -1, 1, 0, 0,
        1, -1, 1, 1,
        1, 1, 1, 0
      ]),
      gl.STATIC_DRAW
    );
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);

    gl.useProgram(this.program);
    const textureLocation = gl.getUniformLocation(this.program, "u_texture");
    if (textureLocation) {
      gl.uniform1i(textureLocation, 0);
    }

    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, 0);
    gl.disable(gl.BLEND);
    gl.disable(gl.DITHER);
    gl.disable(gl.DEPTH_TEST);
    gl.disable(gl.STENCIL_TEST);
    gl.clearColor(0, 0, 0, 1);

    this.syncViewport(Math.max(1, canvas.width), Math.max(1, canvas.height));
  }

  clear() {
    this.gl.clear(this.gl.COLOR_BUFFER_BIT);
  }

  drawFrame(frame: VideoFrame) {
    this.syncViewport(frame.displayWidth, frame.displayHeight);
    this.gl.activeTexture(this.gl.TEXTURE0);
    this.gl.bindTexture(this.gl.TEXTURE_2D, this.texture);
    this.gl.texImage2D(
      this.gl.TEXTURE_2D,
      0,
      this.gl.RGBA,
      this.gl.RGBA,
      this.gl.UNSIGNED_BYTE,
      frame
    );
    this.gl.useProgram(this.program);
    this.gl.bindVertexArray(this.vertexArray);
    this.gl.drawArrays(this.gl.TRIANGLES, 0, 6);
  }

  private syncViewport(width: number, height: number) {
    const nextWidth = Math.max(1, Math.round(width));
    const nextHeight = Math.max(1, Math.round(height));
    if (this.canvas.width !== nextWidth) {
      this.canvas.width = nextWidth;
    }
    if (this.canvas.height !== nextHeight) {
      this.canvas.height = nextHeight;
    }
    this.gl.viewport(0, 0, nextWidth, nextHeight);
  }
}
