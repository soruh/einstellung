pub trait Config: Sized {

    fn store(&self, receiver: impl ConfigReceiver) ;
    fn load(provider: impl ConfigProvider) -> Self;
    fn merge_into_self(&mut self, other: &Self);
    fn merge(&self, other: &Self) -> Self where Self: Clone{
        let mut res = Self::clone(self);
        res.merge_into_self(other);
        res
    }
}


pub trait ConfigProvider {}
pub trait ConfigReceiver {}


pub struct SerdeProvider;
pub struct SerdeReceiver;

impl ConfigProvider for SerdeProvider {}
impl ConfigReceiver for SerdeReceiver {}


#[cfg(feature = "json")]
pub mod json {

    pub struct JsonProvider;
    pub struct JsonReceiver;
    
    impl ConfigProvider for JsonProvider {
        fn provide() {
            SerdeReceiver::provide(serde_json::do_stuff());
        }
    }
    impl ConfigReceiver for JsonReceiver {
        fn receive() {
            serde_json::do_stuff(SerdeReceiver::receive());
        }
    }
}