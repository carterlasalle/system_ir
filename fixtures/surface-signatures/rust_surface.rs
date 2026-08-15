use std::error::Error;

#[derive(Debug)]
pub struct Incident {
    pub id: String,
}

#[derive(Debug)]
pub enum Severity {
    Low,
    High,
}

pub trait Event: Send + Sync {
    fn severity(&self) -> Severity;
}

pub struct Handler;

impl Handler {
    pub async fn process<T: Event>(
        &self,
        event: T,
    ) -> Result<Incident, Box<dyn Error>>
    where
        T: Send + Sync,
    {
        Ok(Incident {
            id: String::new(),
        })
    }
}
