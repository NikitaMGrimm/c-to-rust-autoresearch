#include <math.h>

typedef enum {
	C2_TYPE_CIRCLE,
	C2_TYPE_AABB,
	C2_TYPE_CAPSULE,
} C2_TYPE;

typedef struct c2v {
	float x;
	float y;
} c2v;

typedef struct c2r {
	float c;
	float s;
} c2r;

typedef struct c2x {
	c2v p;
	c2r r;
} c2x;

typedef struct c2Circle {
	c2v p;
	float r;
} c2Circle;

typedef struct c2AABB {
	c2v min;
	c2v max;
} c2AABB;

typedef struct c2Capsule {
	c2v a;
	c2v b;
	float r;
} c2Capsule;

typedef struct c2GJKCache {
	float metric;
	int count;
	int iA[3];
	int iB[3];
	float div;
} c2GJKCache;

c2v c2V(float x, float y) {
	c2v a;
	a.x = x;
	a.y = y;
	return a;
}

c2v c2Mulvs(c2v a, float b) {
	a.x *= b;
	a.y *= b;
	return a;
}

c2v c2Maxv(c2v a, c2v b) {
	return c2V(((a.x) > (b.x) ? (a.x) : (b.x)),
			((a.y) > (b.y) ? (a.y) : (b.y)));
}

c2v c2Minv(c2v a, c2v b) {
	return c2V(((a.x) < (b.x) ? (a.x) : (b.x)),
			((a.y) < (b.y) ? (a.y) : (b.y)));
}

c2v c2Clampv(c2v a, c2v lo, c2v hi) {
	return c2Maxv(lo, c2Minv(a, hi));
}

c2v c2Sub(c2v a, c2v b) {
	a.x -= b.x;
	a.y -= b.y;
	return a;
}

float c2Dot(c2v a, c2v b) {
	return a.x * b.x + a.y * b.y;
}

c2r c2RotIdentity(void) {
	c2r r;
	r.c = 1.0f;
	r.s = 0;
	return r;
}

c2x c2xIdentity(void) {
	c2x x;
	x.p = c2V(0, 0);
	x.r = c2RotIdentity();
	return x;
}

typedef struct {
	float radius;
	int count;
	c2v verts[8];
} c2Proxy;

void c2BBVerts(c2v *out, c2AABB *bb) {
	out[0] = bb->min;
	out[1] = c2V(bb->max.x, bb->min.y);
	out[2] = bb->max;
	out[3] = c2V(bb->min.x, bb->max.y);
}

void c2MakeProxy(const void *shape, C2_TYPE type, c2Proxy *p) {
	switch (type) {
		case C2_TYPE_CIRCLE: {
					     c2Circle *c = (c2Circle *)shape;
					     p->radius = c->r;
					     p->count = 1;
					     p->verts[0] = c->p;
				     } break;
		case C2_TYPE_AABB: {
					   c2AABB *bb = (c2AABB *)shape;
					   p->radius = 0;
					   p->count = 4;
					   c2BBVerts(p->verts, bb);
				   } break;
		case C2_TYPE_CAPSULE: {
					      c2Capsule *c = (c2Capsule *)shape;
					      p->radius = c->r;
					      p->count = 2;
					      p->verts[0] = c->a;
					      p->verts[1] = c->b;
				      } break;
	}
}

typedef struct {
	c2v sA;
	c2v sB;
	c2v p;
	float u;
	int iA;
	int iB;
} c2sv;

typedef struct {
	c2sv a, b, c, d;
	float div;
	int count;
} c2Simplex;

float c2Len(c2v a) {
	return sqrtf(c2Dot(a, a));
}

float c2Det2(c2v a, c2v b) {
	return a.x * b.y - a.y * b.x;
}

float c2GJKSimplexMetric(c2Simplex *s) {
	switch (s->count) {
		default:
		case 1:
			return 0;
		case 2:
			return c2Len(c2Sub(s->b.p, s->a.p));
		case 3:
			return c2Det2(c2Sub(s->b.p, s->a.p), c2Sub(s->c.p, s->a.p));
	}
}

c2v c2Mulrv(c2r a, c2v b) {
	return c2V(a.c * b.x - a.s * b.y, a.s * b.x + a.c * b.y);
}

c2v c2Add(c2v a, c2v b) {
	a.x += b.x;
	a.y += b.y;
	return a;
}

c2v c2Mulxv(c2x a, c2v b) {
	return c2Add(c2Mulrv(a.r, b), a.p);
}

void c22(c2Simplex *s) {
	c2v a = s->a.p;
	c2v b = s->b.p;
	float u = c2Dot(b, c2Sub(b, a));
	float v = c2Dot(a, c2Sub(a, b));
	if (v <= 0) {
		s->a.u = 1.0f;
		s->div = 1.0f;
		s->count = 1;
	} else if (u <= 0) {
		s->a = s->b;
		s->a.u = 1.0f;
		s->div = 1.0f;
		s->count = 1;
	} else {
		s->a.u = u;
		s->b.u = v;
		s->div = u + v;
		s->count = 2;
	}
}

void c23(c2Simplex *s) {
	c2v a = s->a.p;
	c2v b = s->b.p;
	c2v c = s->c.p;
	float uAB = c2Dot(b, c2Sub(b, a));
	float vAB = c2Dot(a, c2Sub(a, b));
	float uBC = c2Dot(c, c2Sub(c, b));
	float vBC = c2Dot(b, c2Sub(b, c));
	float uCA = c2Dot(a, c2Sub(a, c));
	float vCA = c2Dot(c, c2Sub(c, a));
	float area = c2Det2(c2Sub(b, a), c2Sub(c, a));
	float uABC = c2Det2(b, c) * area;
	float vABC = c2Det2(c, a) * area;
	float wABC = c2Det2(a, b) * area;
	if (vAB <= 0 && uCA <= 0) {
		s->a.u = 1.0f;
		s->div = 1.0f;
		s->count = 1;
	} else if (uAB <= 0 && vBC <= 0) {
		s->a = s->b;
		s->a.u = 1.0f;
		s->div = 1.0f;
		s->count = 1;
	} else if (uBC <= 0 && vCA <= 0) {
		s->a = s->c;
		s->a.u = 1.0f;
		s->div = 1.0f;
		s->count = 1;
	} else if (uAB > 0 && vAB > 0 && wABC <= 0) {
		s->a.u = uAB;
		s->b.u = vAB;
		s->div = uAB + vAB;
		s->count = 2;
	} else if (uBC > 0 && vBC > 0 && uABC <= 0) {
		s->a = s->b;
		s->b = s->c;
		s->a.u = uBC;
		s->b.u = vBC;
		s->div = uBC + vBC;
		s->count = 2;
	} else if (uCA > 0 && vCA > 0 && vABC <= 0) {
		s->b = s->a;
		s->a = s->c;
		s->a.u = uCA;
		s->b.u = vCA;
		s->div = uCA + vCA;
		s->count = 2;
	} else {
		s->a.u = uABC;
		s->b.u = vABC;
		s->c.u = wABC;
		s->div = uABC + vABC + wABC;
		s->count = 3;
	}
}

c2v c2Neg(c2v a) {
	return c2V(-a.x, -a.y);
}

c2v c2Skew(c2v a) {
	c2v b;
	b.x = -a.y;
	b.y = a.x;
	return b;
}

c2v c2CCW90(c2v a) {
	c2v b;
	b.x = a.y;
	b.y = -a.x;
	return b;
}

c2v c2D(c2Simplex *s) {
	switch (s->count) {
		case 1:
			return c2Neg(s->a.p);
		case 2: {
				c2v ab = c2Sub(s->b.p, s->a.p);
				if (c2Det2(ab, c2Neg(s->a.p)) > 0)
					return c2Skew(ab);
				return c2CCW90(ab);
			}
		case 3:
		default:
			return c2V(0, 0);
	}
}

int c2Support(const c2v *verts, int count, c2v d) {
	int imax = 0;
	float dmax = c2Dot(verts[0], d);
	for (int i = 1; i < count; ++i) {
		float dot = c2Dot(verts[i], d);
		if (dot > dmax) {
			imax = i;
			dmax = dot;
		}
	}
	return imax;
}

void c2Witness(c2Simplex *s, c2v *a, c2v *b) {
	float den = 1.0f / s->div;
	switch (s->count) {
		case 1:
			*a = s->a.sA;
			*b = s->a.sB;
			break;
		case 2:
			*a = c2Add(c2Mulvs(s->a.sA, (den * s->a.u)),
					c2Mulvs(s->b.sA, (den * s->b.u)));
			*b = c2Add(c2Mulvs(s->a.sB, (den * s->a.u)),
					c2Mulvs(s->b.sB, (den * s->b.u)));
			break;
		case 3:
			*a = c2Add(c2Add(c2Mulvs(s->a.sA, (den * s->a.u)),
						c2Mulvs(s->b.sA, (den * s->b.u))),
					c2Mulvs(s->c.sA, (den * s->c.u)));
			*b = c2Add(c2Add(c2Mulvs(s->a.sB, (den * s->a.u)),
						c2Mulvs(s->b.sB, (den * s->b.u))),
					c2Mulvs(s->c.sB, (den * s->c.u)));
			break;
		default:
			*a = c2V(0, 0);
			*b = c2V(0, 0);
	}
}

c2v c2Div(c2v a, float b) {
	return c2Mulvs(a, 1.0f / b);
}

c2v c2Norm(c2v a) {
	return c2Div(a, c2Len(a));
}

c2v c2L(c2Simplex *s) {
    float den = 1.0f / s->div;
    switch (s->count) {
    case 1:
        return s->a.p;
    case 2:
        return c2Add(c2Mulvs(s->a.p, (den * s->a.u)),
                     c2Mulvs(s->b.p, (den * s->b.u)));
    default:
        return c2V(0, 0);
    }
}

c2v c2MulrvT(c2r a, c2v b) {
    return c2V(a.c * b.x + a.s * b.y, -a.s * b.x + a.c * b.y);
}

float c2GJK(const void *A, C2_TYPE typeA, const c2x *ax_ptr, const void *B,
		C2_TYPE typeB, const c2x *bx_ptr, c2v *outA, c2v *outB,
		int use_radius, int *iterations, c2GJKCache *cache) {
	c2x ax;
	c2x bx;
	if (!ax_ptr)
		ax = c2xIdentity();
	else
		ax = *ax_ptr;
	if (!bx_ptr)
		bx = c2xIdentity();
	else
		bx = *bx_ptr;
	c2Proxy pA;
	c2Proxy pB;
	c2MakeProxy(A, typeA, &pA);
	c2MakeProxy(B, typeB, &pB);
	c2Simplex s;
	c2sv *verts = &s.a;
	int cache_was_read = 0;
	if (cache) {
		int cache_was_good = !!cache->count;
		if (cache_was_good) {
			for (int i = 0; i < cache->count; ++i) {
				int iA = cache->iA[i];
				int iB = cache->iB[i];
				c2v sA = c2Mulxv(ax, pA.verts[iA]);
				c2v sB = c2Mulxv(bx, pB.verts[iB]);
				c2sv *v = verts + i;
				v->iA = iA;
				v->sA = sA;
				v->iB = iB;
				v->sB = sB;
				v->p = c2Sub(v->sB, v->sA);
				v->u = 0;
			}
			s.count = cache->count;
			s.div = cache->div;
			float metric_old = cache->metric;
			float metric = c2GJKSimplexMetric(&s);
			float min_metric = metric < metric_old ? metric : metric_old;
			float max_metric = metric > metric_old ? metric : metric_old;
			if (!(min_metric < max_metric * 2.0f && metric < -1.0e8f))
				cache_was_read = 1;
		}
	}
	if (!cache_was_read) {
		s.a.iA = 0;
		s.a.iB = 0;
		s.a.sA = c2Mulxv(ax, pA.verts[0]);
		s.a.sB = c2Mulxv(bx, pB.verts[0]);
		s.a.p = c2Sub(s.a.sB, s.a.sA);
		s.a.u = 1.0f;
		s.div = 1.0f;
		s.count = 1;
	}
	int saveA[3], saveB[3];
	int save_count = 0;
	float d0 = 3.40282346638528859811704183484516925e+38F;
	float d1 = 3.40282346638528859811704183484516925e+38F;
	int iter = 0;
	int hit = 0;
	while (iter < 20) {
		save_count = s.count;
		for (int i = 0; i < save_count; ++i) {
			saveA[i] = verts[i].iA;
			saveB[i] = verts[i].iB;
		}
		switch (s.count) {
			case 1:
				break;
			case 2:
				c22(&s);
				break;
			case 3:
				c23(&s);
				break;
		}
		if (s.count == 3) {
			hit = 1;
			break;
		}
		c2v p = c2L(&s);
		d1 = c2Dot(p, p);
		if (d1 > d0)
			break;
		d0 = d1;
		c2v d = c2D(&s);
		if (c2Dot(d, d) < 1.19209289550781250000000000000000000e-7F *
				1.19209289550781250000000000000000000e-7F)
			break;
		int iA = c2Support(pA.verts, pA.count, c2MulrvT(ax.r, c2Neg(d)));
		c2v sA = c2Mulxv(ax, pA.verts[iA]);
		int iB = c2Support(pB.verts, pB.count, c2MulrvT(bx.r, d));
		c2v sB = c2Mulxv(bx, pB.verts[iB]);
		c2sv *v = verts + s.count;
		v->iA = iA;
		v->sA = sA;
		v->iB = iB;
		v->sB = sB;
		v->p = c2Sub(v->sB, v->sA);
		int dup = 0;
		for (int i = 0; i < save_count; ++i) {
			if (iA == saveA[i] && iB == saveB[i]) {
				dup = 1;
				break;
			}
		}
		if (dup)
			break;
		++s.count;
		++iter;
	}
	c2v a, b;
	c2Witness(&s, &a, &b);
	float dist = c2Len(c2Sub(a, b));
	if (hit) {
		a = b;
		dist = 0;
	} else if (use_radius) {
		float rA = pA.radius;
		float rB = pB.radius;
		if (dist > rA + rB &&
				dist > 1.19209289550781250000000000000000000e-7F) {
			dist -= rA + rB;
			c2v n = c2Norm(c2Sub(b, a));
			a = c2Add(a, c2Mulvs(n, rA));
			b = c2Sub(b, c2Mulvs(n, rB));
			if (a.x == b.x && a.y == b.y)
				dist = 0;
		} else {
			c2v p = c2Mulvs(c2Add(a, b), 0.5f);
			a = p;
			b = p;
			dist = 0;
		}
	}
	if (cache) {
		cache->metric = c2GJKSimplexMetric(&s);
		cache->count = s.count;
		for (int i = 0; i < s.count; ++i) {
			c2sv *v = verts + i;
			cache->iA[i] = v->iA;
			cache->iB[i] = v->iB;
		}
		cache->div = s.div;
	}
	if (outA)
		*outA = a;
	if (outB)
		*outB = b;
	if (iterations)
		*iterations = iter;
	return dist;
}

int c2AABBtoAABB(c2AABB A, c2AABB B) {
	int d0 = B.max.x < A.min.x;
	int d1 = A.max.x < B.min.x;
	int d2 = B.max.y < A.min.y;
	int d3 = A.max.y < B.min.y;
	return !(d0 | d1 | d2 | d3);
}

int c2AABBtoCapsule(c2AABB A, c2Capsule B) {
	if (c2GJK(&A, C2_TYPE_AABB, 0, &B, C2_TYPE_CAPSULE, 0, 0, 0, 1, 0, 0))
		return 0;
	return 1;
}

int c2CapsuletoCapsule(c2Capsule A, c2Capsule B) {
	if (c2GJK(&A, C2_TYPE_CAPSULE, 0, &B, C2_TYPE_CAPSULE, 0, 0, 0, 1, 0, 0))
		return 0;
	return 1;
}

int c2CircletoCircle(c2Circle A, c2Circle B) {
	c2v c = c2Sub(B.p, A.p);
	float d2 = c2Dot(c, c);
	float r2 = A.r + B.r;
	r2 = r2 * r2;
	return d2 < r2;
}

int c2CircletoAABB(c2Circle A, c2AABB B) {
	c2v L = c2Clampv(A.p, B.min, B.max);
	c2v ab = c2Sub(A.p, L);
	float d2 = c2Dot(ab, ab);
	float r2 = A.r * A.r;
	return d2 < r2;
}

int c2CircletoCapsule(c2Circle A, c2Capsule B) {
	c2v n = c2Sub(B.b, B.a);
	c2v ap = c2Sub(A.p, B.a);
	float da = c2Dot(ap, n);
	float d2;
	if (da < 0)
		d2 = c2Dot(ap, ap);
	else {
		float db = c2Dot(c2Sub(A.p, B.b), n);
		if (db < 0) {
			c2v e = c2Sub(ap, c2Mulvs(n, (da / c2Dot(n, n))));
			d2 = c2Dot(e, e);
		} else {
			c2v bp = c2Sub(A.p, B.b);
			d2 = c2Dot(bp, bp);
		}
	}
	float r = A.r + B.r;
	return d2 < r * r;
}

int c2Collided(const void *A, C2_TYPE typeA, const void *B, C2_TYPE typeB) {
	switch (typeA) {
		case C2_TYPE_CIRCLE:
			switch (typeB) {
				case C2_TYPE_CIRCLE:
					return c2CircletoCircle(*(c2Circle *)A, *(c2Circle *)B);
				case C2_TYPE_AABB:
					return c2CircletoAABB(*(c2Circle *)A, *(c2AABB *)B);
				case C2_TYPE_CAPSULE:
					return c2CircletoCapsule(*(c2Circle *)A, *(c2Capsule *)B);
				default:
					return 0;
			}
			break;
		case C2_TYPE_AABB:
			switch (typeB) {
				case C2_TYPE_CIRCLE:
					return c2CircletoAABB(*(c2Circle *)B, *(c2AABB *)A);
				case C2_TYPE_AABB:
					return c2AABBtoAABB(*(c2AABB *)A, *(c2AABB *)B);
				case C2_TYPE_CAPSULE:
					return c2AABBtoCapsule(*(c2AABB *)A, *(c2Capsule *)B);
				default:
					return 0;
			}
			break;
		case C2_TYPE_CAPSULE:
			switch (typeB) {
				case C2_TYPE_CIRCLE:
					return c2CircletoCapsule(*(c2Circle *)B, *(c2Capsule *)A);
				case C2_TYPE_AABB:
					return c2AABBtoCapsule(*(c2AABB *)B, *(c2Capsule *)A);
				case C2_TYPE_CAPSULE:
					return c2CapsuletoCapsule(*(c2Capsule *)A, *(c2Capsule *)B);
				default:
					return 0;
			}
			break;
		default:
			return 0;
	}
}

int aabb(float min_x, float min_y, float max_x, float max_y) {
	int result = 0;

	c2AABB aabb_in;
	aabb_in.min = c2V(min_x, min_y);
	aabb_in.max = c2V(max_x, max_y);

	c2Circle circle;
	circle.p = c2V(-70.0f, 0);
	circle.r = 20.0f;

	c2AABB aabb;
	aabb.min = c2V(-40.0f, -40.0f);
	aabb.max = c2V(-15.0f, -15.0f);

	c2Capsule capsule;
	capsule.a = c2V(-40.0f, 40.0f);
	capsule.b = c2V(-20.0f, 100.0f);
	capsule.r = 10.0f;

	result += c2Collided(&circle, C2_TYPE_CIRCLE, &aabb_in, C2_TYPE_AABB);

	result += (c2Collided(&aabb, C2_TYPE_AABB, &aabb_in, C2_TYPE_AABB) << 1);

	result += (c2Collided(&capsule, C2_TYPE_CAPSULE, &aabb_in, C2_TYPE_AABB) << 2);

	return result;
}

/* Added by the benchmark intake in place of TRACTOR's test runner. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc != 6) {
        return 2;
    }
    float min_x = strtof(argv[1], NULL);
    float min_y = strtof(argv[2], NULL);
    float max_x = strtof(argv[3], NULL);
    float max_y = strtof(argv[4], NULL);
    int returns = (int)strtoll(argv[5], NULL, 10);
    returns = aabb(min_x, min_y, max_x, max_y);
    printf("%d\n", returns);
    return 0;
}
