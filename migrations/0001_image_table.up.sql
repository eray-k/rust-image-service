create table image (
  id uuid primary key, 
  user_id uuid not null,
  filepath varchar,
  preview_path varchar
);

create index user_id_index on image (user_id);