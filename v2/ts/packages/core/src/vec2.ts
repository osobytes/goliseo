// Immutable 2D vector. Pure math, usable by every layer.

/** Immutable 2D vector. Pure math, usable by every layer. */
export class Vec2 {
  readonly x: number;
  readonly y: number;

  constructor(x = 0, y = 0) {
    this.x = x;
    this.y = y;
  }

  /** Component-wise addition. */
  add(o: Vec2): Vec2 {
    return new Vec2(this.x + o.x, this.y + o.y);
  }

  /** Component-wise subtraction. */
  sub(o: Vec2): Vec2 {
    return new Vec2(this.x - o.x, this.y - o.y);
  }

  /** Scales both components by `s`. */
  scale(s: number): Vec2 {
    return new Vec2(this.x * s, this.y * s);
  }

  /** Euclidean length. */
  length(): number {
    return Math.sqrt(this.x * this.x + this.y * this.y);
  }

  /** Unit-length vector in the same direction, or the zero vector (no NaN). */
  normalized(): Vec2 {
    const len = this.length();
    if (len === 0) {
      return new Vec2(0, 0);
    }
    return new Vec2(this.x / len, this.y / len);
  }

  /** Euclidean distance to `o`. */
  dist(o: Vec2): number {
    return this.sub(o).length();
  }
}
