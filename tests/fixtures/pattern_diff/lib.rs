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
