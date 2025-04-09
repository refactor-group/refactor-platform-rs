use crate::error::Error;
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, Value,
};
use std::collections::HashMap;
/// Updates an existing record in the database using a map of column names to values.
///
/// This function provides a flexible way to update only specific fields of an entity
/// without having to provide all fields. It takes an active model and an update map,
/// and only modifies the fields specified in the map.
///
/// # Type Parameters
///
/// * `A` - The ActiveModel type that implements ActiveModelTrait and ActiveModelBehavior
/// * `C` - The Column type that implements ColumnTrait
///
/// # Arguments
///
/// * `db` - A reference to the database connection
/// * `active_model` - The active model to update
/// * `update_map` - A map of column names to their new values
///
/// # Returns
///
/// Returns a Result containing either the updated Model or an Error
pub async fn update<A, C>(
    db: &DatabaseConnection,
    mut active_model: A,
    update_map: UpdateMap,
) -> Result<<A::Entity as EntityTrait>::Model, Error>
where
    A: ActiveModelTrait + ActiveModelBehavior + Send,
    C: ColumnTrait,
    A::Entity: EntityTrait<Column = C>,
    <A::Entity as EntityTrait>::Model: IntoActiveModel<A>,
{
    for column in C::iter() {
        if let Some(value) = update_map.get(&column.to_string()) {
            active_model.set(column, value.clone());
        }
    }
    Ok(active_model.update(db).await?)
}

/// A map structure that holds column names and their corresponding values for updates.
///
/// This structure provides a flexible way to specify which fields should be updated
/// and their new values. It's designed to work with SeaORM's Value type and supports
/// optional values to handle nullable fields.
#[derive(Default)]
pub struct UpdateMap {
    map: HashMap<String, Option<Value>>,
}

impl UpdateMap {
    /// Creates a new empty UpdateMap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Retrieves a value from the map by its key.
    ///
    /// Returns an Option containing a reference to the Value if it exists,
    /// or None if the key is not found or the value is None.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key).and_then(|opt| opt.as_ref())
    }

    /// Removes a key-value pair from the map.
    ///
    /// Returns the removed value if it exists, or None if the key is not found.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.map.remove(key).and_then(|opt| opt)
    }

    /// Inserts a key-value pair into the map.
    ///
    /// If the key already exists, the value will be overwritten.
    pub fn insert(&mut self, key: String, value: Option<Value>) {
        self.map.insert(key, value);
    }
}

/// A trait that allows types to be converted into an UpdateMap.
///
/// This trait provides a way to convert various types into an UpdateMap,
/// making it easier to create update maps from different data structures.
pub trait IntoUpdateMap {
    /// Converts the implementing type into an UpdateMap.
    fn into_update_map(self) -> UpdateMap;
}
