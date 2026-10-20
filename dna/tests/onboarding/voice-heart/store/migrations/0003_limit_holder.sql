alter table "limit" add constraint limit_holder check (
  (scope = 'project' and project is not null and key is null and model_id is null and account is null)
  or (scope = 'key' and key is not null and project is null and model_id is null and account is null)
  or (scope = 'account' and account is not null and project is null and key is null and model_id is null)
  or (scope = 'model' and project is not null and model_id is not null and key is null and account is null)
);
