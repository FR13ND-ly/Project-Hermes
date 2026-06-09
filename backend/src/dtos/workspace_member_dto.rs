use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMemberRequest {
    pub email: String,
    pub role_name: String, // 'owner', 'admin', 'developer', 'viewer'
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMemberRoleRequest {
    pub role_name: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMemberResponse {
    pub user_id: Uuid,
    pub email: String,
    pub username: String,
    pub role_name: String,
}
