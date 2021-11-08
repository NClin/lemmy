use crate::PerformCrud;
use actix_web::web::Data;
use lemmy_api_common::{
  blocking,
  community::{CommunityResponse, EditCommunity},
  get_local_user_view_from_jwt,
};
use lemmy_apub::protocol::activities::community::update::UpdateCommunity;
use lemmy_db_schema::{
  diesel_option_overwrite_to_url,
  naive_now,
  newtypes::PersonId,
  source::community::{Community, CommunityForm},
  traits::Crud,
};
use lemmy_db_views_actor::community_moderator_view::CommunityModeratorView;
use lemmy_utils::{utils::check_slurs_opt, ApiError, ConnectionId, LemmyError};
use lemmy_websocket::{send::send_community_ws_message, LemmyContext, UserOperationCrud};

#[async_trait::async_trait(?Send)]
impl PerformCrud for EditCommunity {
  type Response = CommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    websocket_id: Option<ConnectionId>,
  ) -> Result<CommunityResponse, LemmyError> {
    let data: &EditCommunity = self;
    let local_user_view =
      get_local_user_view_from_jwt(&data.auth, context.pool(), context.secret()).await?;

    check_slurs_opt(&data.title, &context.settings().slur_regex())?;
    check_slurs_opt(&data.description, &context.settings().slur_regex())?;

    // Verify its a mod (only mods can edit it)
    let community_id = data.community_id;
    let mods: Vec<PersonId> = blocking(context.pool(), move |conn| {
      CommunityModeratorView::for_community(conn, community_id)
        .map(|v| v.into_iter().map(|m| m.moderator.id).collect())
    })
    .await??;
    if !mods.contains(&local_user_view.person.id) {
      return Err(ApiError::err_plain("not_a_moderator").into());
    }

    let community_id = data.community_id;
    let read_community = blocking(context.pool(), move |conn| {
      Community::read(conn, community_id)
    })
    .await??;

    let icon = diesel_option_overwrite_to_url(&data.icon)?;
    let banner = diesel_option_overwrite_to_url(&data.banner)?;

    let community_form = CommunityForm {
      name: read_community.name,
      title: data.title.to_owned().unwrap_or(read_community.title),
      description: data.description.to_owned(),
      icon,
      banner,
      nsfw: data.nsfw,
      updated: Some(naive_now()),
      ..CommunityForm::default()
    };

    let community_id = data.community_id;
    let updated_community = blocking(context.pool(), move |conn| {
      Community::update(conn, community_id, &community_form)
    })
    .await?
    .map_err(|e| ApiError::err("couldnt_update_community", e))?;

    UpdateCommunity::send(
      updated_community.into(),
      &local_user_view.person.into(),
      context,
    )
    .await?;

    let op = UserOperationCrud::EditCommunity;
    send_community_ws_message(data.community_id, op, websocket_id, None, context).await
  }
}
