-- Authorization audit context only. linked_by names the designated owner when assigned by an operator.
ALTER TABLE project_workspaces ADD COLUMN linked_via TEXT NOT NULL DEFAULT 'user' CHECK (linked_via IN ('user','operator'));
