pub mod context;
pub mod hedger;
pub mod info;
pub mod inventory;
pub mod maker;
pub mod oracle;
pub mod order_manager;
pub mod orders;
pub mod strategy;

/// Defines shared functionality that different components should implement in order to be identifiable.
pub trait Identifier {
    /// Gets the symbol this component represents.
    fn symbol(&self) -> &str;
}
