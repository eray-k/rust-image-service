-- Key values stay as they are: the upload dir they were relative to isn't recorded
alter table image rename column storage_key to filepath;
