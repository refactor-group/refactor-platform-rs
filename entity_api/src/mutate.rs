use crate::error::{EntityApiErrorKind, Error};
use sea_orm::{
    ActiveModelBehavior, ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait,
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
    db: &impl ConnectionTrait,
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
        if let Some(value) = update_map.get_value(&column.to_string()) {
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
#[derive(Default, Debug)]
pub struct UpdateMap {
    map: HashMap<String, Option<Value>>,
}

impl UpdateMap {
    /// Creates a new empty UpdateMap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a key-value pair into the map.
    ///
    /// If the key already exists, the value will be overwritten.
    pub fn insert(&mut self, key: String, value: Option<Value>) {
        self.map.insert(key, value);
    }

    /// Retrieves a value from the map by its key.
    ///
    /// Returns an Option containing a reference to the Value if it exists,
    /// or None if the key is not found or the value is None.
    pub fn get_value(&self, key: &str) -> Option<&Value> {
        self.map.get(key).and_then(|opt| opt.as_ref())
    }

    /// Removes a value from the update map and returns it.
    /// Returns Error if the key doesn't exist or the value is not a valid string.
    pub fn remove(&mut self, key: &str) -> Result<String, Error> {
        self.map
            .remove(key)
            .ok_or_else(|| Error {
                source: None,
                error_kind: EntityApiErrorKind::Other("Key not found".to_string()),
            })
            .and_then(|v| match v {
                Some(Value::String(Some(boxed_str))) => Ok((*boxed_str).clone()),
                _ => Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::Other("Value is not a string".to_string()),
                }),
            })
    }

    /// Gets a value from the update map without removing it.
    /// Returns Error if the key doesn't exist or the value is not a valid string.
    pub fn get(&self, key: &str) -> Result<&String, Error> {
        self.map
            .get(key)
            .and_then(|opt| opt.as_ref())
            .ok_or_else(|| Error {
                source: None,
                error_kind: EntityApiErrorKind::Other("Key not found".to_string()),
            })
            .and_then(|v| match v {
                Value::String(Some(boxed_str)) => Ok(&**boxed_str),
                _ => Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::Other("Value is not a string".to_string()),
                }),
            })
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
