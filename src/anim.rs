//! A damped spring: gives the island its liquid, slightly elastic shape changes.

#[derive(Clone, Copy)]
pub struct Spring {
    pub v: f32,
    pub t: f32,
    vel: f32,
    k: f32,
    c: f32,
    eps: f32,
}

impl Spring {
    pub fn new(v: f32, stiffness: f32, damping: f32, eps: f32) -> Self {
        Self { v, t: v, vel: 0.0, k: stiffness, c: damping, eps }
    }

    pub fn set(&mut self, t: f32) {
        self.t = t;
    }

    pub fn snap(&mut self, v: f32) {
        self.v = v;
        self.t = v;
        self.vel = 0.0;
    }

    pub fn step(&mut self, dt: f32) {
        let mut rem = dt;
        while rem > 0.0 {
            let h = rem.min(1.0 / 240.0);
            let a = -self.k * (self.v - self.t) - self.c * self.vel;
            self.vel += a * h;
            self.v += self.vel * h;
            rem -= h;
        }
        if self.settled() {
            self.v = self.t;
            self.vel = 0.0;
        }
    }

    pub fn settled(&self) -> bool {
        (self.v - self.t).abs() < self.eps && self.vel.abs() < self.eps * 4.0
    }
}
