pub fn process_request() {}

pub fn other() {
    process_request();
}

pub struct App {}

pub struct AppContext {}

impl App {
    pub fn tick(&self) {
        self.helper();
    }

    pub fn helper(&self) {}
}

pub fn demo(app: App) {
    app.tick();
}

pub fn guard(x: i32) -> i32 {
    if x > 0 { return x; }
    if x < -10 {
        let y = -x;
        return y + 1;
    }
    0
}
