Feature: Operator
  An operator of a Rubato server creates tenants, attaches GitHub App installations to them and
  names their first admins (ADR 0087, 0088). An operator sees names and counts, never a tenant's
  data, and need be a member of none.

  @admin.only-operators
  Scenario: Only an operator reaches the operator's page
    Given a person who is not an operator
    Then they see no Operator button
    And the operator's routes answer them as if they did not exist

  @admin.tenants-and-installations
  Scenario: The operator sees tenants and installations as names and counts
    When an operator opens the Operator page
    Then each tenant shows its name, its members, admins and workspaces as counts, and its installations
    And each installation of the GitHub App shows its account and whether it is attached, suspended, detached or not attached

  @admin.no-tenant-data
  Scenario: An operator who is in no tenant sees no tenant's data
    Given an operator who is a member of no tenant
    When they open Rubato
    Then they are told they are in no tenant, and the Operator page is open
    And no tenant's map, issues or plans can be read by them

  @admin.create-tenant
  Scenario: The operator creates a tenant with its first admin
    When an operator creates a tenant with an id, a name and the GitHub login of its first admin
    Then the tenant exists with that person as its admin
    And nothing is created when the login is unknown or an installation is taken

  @admin.attach
  Scenario: An installation is attached to one tenant
    When an operator attaches an installation that is not attached
    Then the tenant's workspaces may name its repositories
    And attaching it to a second tenant is refused, naming the tenant that holds it

  @admin.detach-keeps-rows
  Scenario: Detaching keeps the rows, hidden, until they are purged
    Given an installation no workspace of its tenant is on
    When an operator detaches it
    Then its repositories are no longer read
    And their rows are kept and shown to nobody

  @admin.detach-refused-in-use
  Scenario: Detaching is refused while a workspace is on the installation's repositories
    When an operator detaches an installation a workspace is on
    Then it is refused, and the page names the workspace to delete first

  @admin.purge-names-rows-first
  Scenario: A purge shows what it would delete before it deletes
    Given a detached installation
    When an operator presses Purge
    Then the page shows how many rows of which tables would be deleted
    And nothing is deleted until the operator confirms

  @admin.delete-empty-tenant
  Scenario: Only an empty tenant is deleted
    When an operator deletes a tenant with a workspace or an attached installation
    Then it is refused
    And a tenant with neither is deleted, its members with it

  @admin.name-admin
  Scenario: The operator can always name an admin
    Given a tenant whose admins cannot be reached
    When an operator names a GitHub login as its admin
    Then that person is an admin of the tenant
