/* Hoist user `varying` declarations above main() before WebGL sees them (#391).
 *
 * WHY THIS EXISTS. LÖVE stitches every shader as: its own boilerplate, its own
 * `void main()`, a `#line 1`, and only then the user's code. A user-declared
 * `varying` therefore always sits BELOW `main()` in the GLSL ES 1.00 source
 * that reaches WebGL. That is legal -- the declaration still precedes its only
 * use, inside the user's `position()`/`effect()` -- and desktop GL, Chrome and
 * Mesa all accept it.
 *
 * Firefox's WebGL shader translator does not. It applies the WebGL-mandated
 * "initialise every output variable" pass by injecting the initialisation at
 * the TOP of `main()` while leaving declarations in source order, so it emits
 * a write to `webgl_<hash>` at line 19 and the matching `out vec3 webgl_<hash>`
 * at line 29. The output is invalid GLSL and the driver rejects it -- NVIDIA
 * with `error C1503: undefined variable`, Mesa with `undeclared`. The bug is in
 * what Firefox emits, so it is driver-independent, and it reproduces in four
 * lines of plain WebGL with no LÖVE and no wasm.
 *
 * Upstream: https://bugzilla.mozilla.org/show_bug.cgi?id=2039887 (filed
 * 2026-05-15 by the Castle Game Engine developer, same diagnosis, still broken
 * in Nightly 153.0b13 after Mozilla's ANGLE update in bug 1908744). Castle
 * Game Engine shipped an engine-level hoisting workaround rather than wait;
 * this is the same workaround shape, applied at the WebGL boundary instead of
 * inside the engine, because LÖVE's stitcher owns the placement and we do not
 * fork it.
 *
 * WHAT IT DOES. `shaderSource` is wrapped, and full-line top-level `varying`
 * declarations that appear after the first `void main(` are moved above it.
 * Everything else is returned byte-for-byte unchanged.
 *
 * LINE NUMBERS ARE PRESERVED. LÖVE maps compile errors back to user code with
 * the `#line 1` it emits just before that code, so a transform that shifts
 * lines silently corrupts every shader error message a developer ever reads.
 * The declaration lines are blanked rather than deleted, and the hoisted text
 * is prepended to the existing `void main(` line rather than inserted as new
 * lines, so in the common case the output has exactly as many lines as the
 * input and every line keeps its number. Only a declaration that sits inside a
 * preprocessor conditional needs its directives replayed on their own lines,
 * and that insertion does shift the lines below it -- so it is taken ONLY when
 * a `#line` directive exists below `main()` to reset numbering, which is what
 * makes the shift unobservable in user-code error messages. A source without
 * one is returned unchanged rather than silently renumbered.
 *
 * It is applied in every browser, not only Firefox. Hoisting is semantically
 * neutral, and one shader source for all runtimes is worth more than skipping
 * a no-op: browser-sniffing would leave Chrome running GLSL that Firefox never
 * sees. Measured: Chrome passes identically with and without it.
 */
(function (root) {
  "use strict";

  /* A whole line that is nothing but one `varying` declaration. Anchored at
   * both ends against the comment-stripped line, so `varyingFoo`, a mid-line
   * declaration and anything with a brace or a call in it are all rejected. */
  var DECLARATION = /^\s*varying\s+[^;{}()]*;\s*$/;
  var MAIN = /\bvoid\s+main\s*\(/;
  var CONDITIONAL_OPEN = /^\s*#\s*(if|ifdef|ifndef)\b/;
  var CONDITIONAL_BRANCH = /^\s*#\s*(elif|else)\b/;
  var CONDITIONAL_END = /^\s*#\s*endif\b/;
  var MARKER = "/* #391 hoisted */";

  /* Blank out every comment, keeping line count and column count intact, so
   * that all structural decisions below are made on code and a `varying`
   * inside a comment can never be hoisted into live code. GLSL ES 1.00 has no
   * string literals, so comments are the only quoting construct there is. */
  function maskComments(lines) {
    var masked = [];
    var inBlock = false;
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      var out = "";
      var j = 0;
      while (j < line.length) {
        if (inBlock) {
          if (line.charAt(j) === "*" && line.charAt(j + 1) === "/") {
            inBlock = false;
            out += "  ";
            j += 2;
          } else {
            out += " ";
            j += 1;
          }
        } else if (line.charAt(j) === "/" && line.charAt(j + 1) === "*") {
          inBlock = true;
          out += "  ";
          j += 2;
        } else if (line.charAt(j) === "/" && line.charAt(j + 1) === "/") {
          break;
        } else {
          out += line.charAt(j);
          j += 1;
        }
      }
      masked.push(out);
    }
    return masked;
  }

  function countBraces(text) {
    var depth = 0;
    for (var i = 0; i < text.length; i++) {
      if (text.charAt(i) === "{") {
        depth += 1;
      } else if (text.charAt(i) === "}") {
        depth -= 1;
      }
    }
    return depth;
  }

  function findMain(code) {
    for (var i = 0; i < code.length; i++) {
      if (MAIN.test(code[i])) {
        return i;
      }
    }
    return -1;
  }

  /* Walk the source below main() and collect the declarations to move, each
   * with the preprocessor branch it was found in. Returns null to mean "leave
   * this source completely alone", which is the answer for anything the walk
   * does not fully understand. */
  function plan(lines, code, mainLine) {
    var found = [];
    var braces = countBraces(code[mainLine]);
    var stack = [];
    for (var i = mainLine + 1; i < lines.length; i++) {
      var line = code[i];
      if (CONDITIONAL_OPEN.test(line)) {
        stack.push([lines[i].trim()]);
        continue;
      }
      if (CONDITIONAL_BRANCH.test(line)) {
        if (stack.length === 0) {
          return null; /* unbalanced: not ours to rewrite */
        }
        stack[stack.length - 1].push(lines[i].trim());
        continue;
      }
      if (CONDITIONAL_END.test(line)) {
        if (stack.length === 0) {
          return null;
        }
        stack.pop();
        continue;
      }
      if (braces === 0 && DECLARATION.test(line)) {
        var branch = [];
        for (var s = 0; s < stack.length; s++) {
          branch = branch.concat(stack[s]);
        }
        found.push({ line: i, text: line.trim(), branch: branch, depth: stack.length });
        continue;
      }
      braces += countBraces(line);
      if (braces < 0) {
        return null;
      }
    }
    if (braces !== 0 || stack.length !== 0) {
      return null;
    }
    return found;
  }

  /**
   * @param {string} source GLSL as LÖVE stitched it.
   * @returns {string} the same source with top-level `varying` declarations
   *   below `main()` moved above it, or the identical string if there are none.
   */
  function hoist(source) {
    if (typeof source !== "string" || source.indexOf("varying") < 0) {
      return source;
    }
    var lines = source.split("\n");
    var code = maskComments(lines);
    var mainLine = findMain(code);
    if (mainLine < 0) {
      return source;
    }
    var found = plan(lines, code, mainLine);
    if (found === null || found.length === 0) {
      return source;
    }

    var unconditional = true;
    for (var i = 0; i < found.length; i++) {
      if (found[i].depth > 0) {
        unconditional = false;
      }
    }

    /* A guarded declaration cannot ride on the `void main(` line, because a
     * preprocessor directive must own its line. That means inserting lines, and
     * inserting lines moves every line number below the insertion point --
     * harmless only because LÖVE's `#line 1` resets numbering for the user code
     * that error messages actually refer to. Without such a directive the
     * promise this file makes about line numbers would quietly stop being true
     * while still emitting valid GLSL, which is the worst way to be wrong.
     * So: no `#line` below `main()`, no rewrite. Unreachable through LÖVE,
     * which always emits one; the check is for whatever is added later. */
    if (!unconditional) {
      var canRenumber = false;
      for (var m = mainLine; m < code.length; m++) {
        if (/^\s*#\s*line\b/.test(code[m])) {
          canRenumber = true;
          break;
        }
      }
      if (!canRenumber) {
        return source;
      }
    }

    for (var b = 0; b < found.length; b++) {
      lines[found[b].line] = "";
    }

    if (unconditional) {
      /* No directives to replay, so the whole block fits on the existing
       * `void main(` line and not one line number moves. */
      var inline = [];
      for (var j = 0; j < found.length; j++) {
        inline.push(found[j].text);
      }
      lines[mainLine] = MARKER + " " + inline.join(" ") + " " + lines[mainLine];
      return lines.join("\n");
    }

    /* A declaration guarded by `#ifdef` keeps its guard: the directives seen at
     * each open level up to the declaration are replayed verbatim, which
     * reproduces the exact branch it was written in (including `#else`), so the
     * hoist can never turn a disabled declaration into a live one. */
    var block = [MARKER];
    for (var k = 0; k < found.length; k++) {
      block = block.concat(found[k].branch);
      block.push(found[k].text);
      for (var e = 0; e < found[k].depth; e++) {
        block.push("#endif");
      }
    }
    var head = lines.slice(0, mainLine);
    var tail = lines.slice(mainLine);
    return head.concat(block, tail).join("\n");
  }

  /**
   * Wrap `shaderSource` on whichever WebGL context prototypes exist. Idempotent:
   * a second call on an already-wrapped prototype does nothing, so loading the
   * bootstrap twice cannot stack transforms.
   *
   * @param {object} scope object carrying the WebGL constructors (default: root).
   * @returns {string[]} the names of the prototypes newly wrapped.
   */
  function install(scope) {
    var host = scope || root;
    var wrapped = [];
    ["WebGLRenderingContext", "WebGL2RenderingContext"].forEach(function (name) {
      var constructor = host[name];
      if (!constructor || !constructor.prototype || !constructor.prototype.shaderSource) {
        return;
      }
      var proto = constructor.prototype;
      if (proto.__goliseo_shader_hoist__) {
        return;
      }
      var original = proto.shaderSource;
      proto.shaderSource = function (shader, source) {
        return original.call(this, shader, hoist(source));
      };
      proto.__goliseo_shader_hoist__ = true;
      wrapped.push(name);
    });
    return wrapped;
  }

  root.GoliseoShaderHoist = {
    hoist: hoist,
    install: install,
    marker: MARKER,
    /* Named prototypes wrapped at load time, so a page can prove in one
     * expression that the workaround is actually active. */
    installed: install()
  };
})(typeof window !== "undefined" ? window : globalThis);
