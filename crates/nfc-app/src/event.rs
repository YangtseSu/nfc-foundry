#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationState {
    Idle,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskEvent {
    pub task: String,
    pub state: OperationState,
    pub progress: u8,
    pub message: String,
}
