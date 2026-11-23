-- Images are addressed by storage key (`<uuid>.<ext>`) instead of a path under the upload dir
alter table image rename column filepath to storage_key;
update image set storage_key = regexp_replace(storage_key, '^.*/', '') where storage_key like '%/%';
