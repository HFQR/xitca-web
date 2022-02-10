use tokio::sync::mpsc::Sender;

pub struct Message(pub Sender<()>);
