use async_graphql::ResultExt;
use async_graphql::{Context, Object};

use qm_entity::ctx::InstitutionFilter;
use qm_entity::ctx::MutationContext;
use qm_entity::ctx::OrganizationFilter;
use qm_entity::err;
use qm_entity::error::EntityResult;
use qm_entity::ids::InstitutionId;
use qm_entity::ids::OrganizationId;
use qm_entity::list::ListCtx;
use qm_entity::model::ListFilter;
use qm_entity::Create;
use qm_mongodb::DB;

use crate::context::RelatedAccessLevel;
use crate::context::RelatedAuth;
use crate::context::RelatedPermission;
use crate::context::RelatedResource;
use crate::context::RelatedStorage;
use crate::marker::Marker;
use crate::model::CreateInstitutionInput;
use crate::model::CreateUserInput;
use crate::model::Institution;
use crate::model::{InstitutionData, InstitutionList, UpdateInstitutionInput};
use crate::roles;
use crate::schema::auth::AuthCtx;

pub const DEFAULT_COLLECTION: &str = "institutions";

pub trait InstitutionDB: AsRef<DB> {
    fn collection(&self) -> &str {
        DEFAULT_COLLECTION
    }
    fn institutions(&self) -> qm_entity::Collection<Institution> {
        let collection = self.collection();
        qm_entity::Collection(self.as_ref().get().collection::<Institution>(collection))
    }
}

pub struct Ctx<'a, Auth, Store, AccessLevel, Resource, Permission>(
    pub AuthCtx<'a, Auth, Store, AccessLevel, Resource, Permission>,
)
where
    Auth: RelatedAuth<AccessLevel, Resource, Permission>,
    Store: RelatedStorage,
    AccessLevel: RelatedAccessLevel,
    Resource: RelatedResource,
    Permission: RelatedPermission;
impl<'a, Auth, Store, AccessLevel, Resource, Permission>
    Ctx<'a, Auth, Store, AccessLevel, Resource, Permission>
where
    Auth: RelatedAuth<AccessLevel, Resource, Permission>,
    Store: RelatedStorage,
    AccessLevel: RelatedAccessLevel,
    Resource: RelatedResource,
    Permission: RelatedPermission,
{
    pub async fn list(
        &self,
        filter: Option<ListFilter>,
    ) -> async_graphql::FieldResult<InstitutionList> {
        ListCtx::new(self.0.store.institutions())
            .list(filter)
            .await
            .extend()
    }

    pub async fn create(&self, institution: InstitutionData) -> EntityResult<Institution> {
        let OrganizationId { cid, id: oid } = institution.0.clone();
        let name = institution.1.clone();
        let lock_key = format!(
            "v1_institution_lock_{}_{}_{name}",
            cid.to_hex(),
            oid.to_hex()
        );
        let lock = self.0.store.redis().lock(&lock_key, 5000, 20, 250).await?;
        let (result, exists) = async {
            EntityResult::Ok(
                if let Some(item) = self
                    .0
                    .store
                    .institutions()
                    .by_field_with_customer_filter(&cid, "name", &name)
                    .await?
                {
                    (item, true)
                } else {
                    let result = self
                        .0
                        .store
                        .institutions()
                        .save(institution.create(&self.0.auth)?)
                        .await?;
                    let access = qm_role::Access::new(AccessLevel::institution())
                        .with_fmt_id(result.id.as_institution_id().as_ref())
                        .to_string();
                    let roles =
                        roles::ensure(self.0.store.keycloak(), Some(access).into_iter()).await?;
                    let cache = self.0.store.cache();
                    cache
                        .customer()
                        .new_institution(self.0.store.redis().as_ref(), result.clone())
                        .await?;
                    cache
                        .user()
                        .new_roles(self.0.store, self.0.store.redis().as_ref(), roles)
                        .await?;
                    if let Some(producer) = self.0.store.mutation_event_producer() {
                        producer
                            .create_event(
                                &qm_kafka::producer::EventNs::Institution,
                                InstitutionDB::collection(self.0.store),
                                &result,
                            )
                            .await?;
                    }
                    (result, false)
                },
            )
        }
        .await?;
        self.0.store.redis().unlock(&lock_key, &lock.id).await?;
        if exists {
            return err!(name_conflict::<Institution>(name));
        }
        Ok(result)
    }
}

pub struct InstitutionQueryRoot<Auth, Store, AccessLevel, Resource, Permission> {
    _marker: Marker<Auth, Store, AccessLevel, Resource, Permission>,
}

impl<Auth, Store, AccessLevel, Resource, Permission> Default
    for InstitutionQueryRoot<Auth, Store, AccessLevel, Resource, Permission>
{
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

#[Object]
impl<Auth, Store, AccessLevel, Resource, Permission>
    InstitutionQueryRoot<Auth, Store, AccessLevel, Resource, Permission>
where
    Auth: RelatedAuth<AccessLevel, Resource, Permission>,
    Store: RelatedStorage,
    AccessLevel: RelatedAccessLevel,
    Resource: RelatedResource,
    Permission: RelatedPermission,
{
    async fn institution_by_id(
        &self,
        _ctx: &Context<'_>,
        _id: InstitutionId,
    ) -> async_graphql::FieldResult<Option<Institution>> {
        // Ok(InstitutionCtx::<Auth, Store>::from_graphql(ctx)
        //     .await?
        //     .by_id(&id)
        //     .await?)
        unimplemented!()
    }

    async fn institutions(
        &self,
        ctx: &Context<'_>,
        filter: Option<ListFilter>,
    ) -> async_graphql::FieldResult<InstitutionList> {
        Ctx(
            AuthCtx::<'_, Auth, Store, AccessLevel, Resource, Permission>::new_with_role(
                ctx,
                (Resource::institution(), Permission::list()),
            )
            .await?,
        )
        .list(filter)
        .await
        .extend()
    }
}

pub struct InstitutionMutationRoot<Auth, Store, AccessLevel, Resource, Permission> {
    _marker: Marker<Auth, Store, AccessLevel, Resource, Permission>,
}

impl<Auth, Store, AccessLevel, Resource, Permission> Default
    for InstitutionMutationRoot<Auth, Store, AccessLevel, Resource, Permission>
{
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

#[Object]
impl<Auth, Store, AccessLevel, Resource, Permission>
    InstitutionMutationRoot<Auth, Store, AccessLevel, Resource, Permission>
where
    Auth: RelatedAuth<AccessLevel, Resource, Permission>,
    Store: RelatedStorage,
    AccessLevel: RelatedAccessLevel,
    Resource: RelatedResource,
    Permission: RelatedPermission,
{
    async fn create_institution(
        &self,
        ctx: &Context<'_>,
        context: OrganizationFilter,
        input: CreateInstitutionInput,
    ) -> async_graphql::FieldResult<Institution> {
        let result = Ctx(
            AuthCtx::<Auth, Store, AccessLevel, Resource, Permission>::mutate_with_role(
                ctx,
                MutationContext::Organization(context.clone()),
                (Resource::institution(), Permission::create()),
            )
            .await?,
        )
        .create(InstitutionData(context.into(), input.name))
        .await
        .extend()?;
        if let Some(user) = input.initial_user {
            crate::schema::user::Ctx(
                AuthCtx::<'_, Auth, Store, AccessLevel, Resource, Permission>::new_with_role(
                    ctx,
                    (Resource::user(), Permission::create()),
                )
                .await?,
            )
            .create(CreateUserInput {
                access: qm_role::Access::new(AccessLevel::institution())
                    .with_fmt_id(result.id.as_institution_id().as_ref())
                    .to_string(),
                user,
                group: Auth::create_institution_owner_group().name,
                context: qm_entity::ctx::ContextFilterInput::Institution(InstitutionFilter {
                    customer: result.id.cid.clone().unwrap(),
                    organization: result.id.oid.clone().unwrap(),
                    institution: result.id.id.clone().unwrap(),
                }),
            })
            .await
            .extend()?;
        }
        Ok(result)
    }

    async fn update_institution(
        &self,
        _ctx: &Context<'_>,
        _input: UpdateInstitutionInput,
    ) -> async_graphql::FieldResult<Institution> {
        // Ok(InstitutionCtx::<Auth, Store>::from_graphql(ctx)
        //     .await?
        //     .update(&input)
        //     .await?)
        unimplemented!()
    }

    async fn remove_institutions(
        &self,
        _ctx: &Context<'_>,
        _ids: Vec<InstitutionId>,
    ) -> async_graphql::FieldResult<usize> {
        // Ok(InstitutionCtx::<Auth, Store>::from_graphql(ctx)
        //     .await?
        //     .remove(&ids)
        //     .await?)
        unimplemented!()
    }
}