-- Rejections keep their code only (audit HTTP-6, PROTO-4). The message can
-- quote the decrypted payload, and it already travels inside the signed
-- rejection, encrypted to the proposer; nothing read the column.
alter table audit drop column message;
