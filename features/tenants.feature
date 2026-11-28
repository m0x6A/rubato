Feature: Tenants
  Several teams share one Rubato without seeing each other (ADR 0087). A tenant owns GitHub App
  installations, workspaces and members; a person may be in several. Being invited is the only
  way in (ADR 0088).

  @tenants.switch
  Scenario: A person in two tenants switches between them
    Given a person who is a member of the tenants "acme" and "bcorp"
    When they pick "bcorp" in the tenant switcher
    Then the page shows bcorp's map, people and plans
    And nothing of acme is on the page

  @tenants.one-tenant-no-switcher
  Scenario: A person in one tenant sees no switcher
    Given a person who is a member of one tenant
    Then no tenant switcher is drawn

  @tenants.unknown-is-not-found
  Scenario: A tenant one is not in answers as one that does not exist
    Given a person who is a member of "acme" only
    When they ask for anything of "bcorp"
    Then the answer is the one for a tenant that does not exist

  @tenants.other-tenants-things-not-found
  Scenario: Another tenant's repository, launch, draft or room is not found inside one's own tenant
    Given a person who is a member of "acme" only
    When they name a repository, launch, draft or room of "bcorp" in a request to "acme"
    Then the answer is "not found"
    And nothing of bcorp is read or changed

  @tenants.members-list
  Scenario: Every member sees who is in the tenant
    When a member opens Members
    Then they see each member with their role
    And a person who was invited and has not logged in yet is marked "invited"

  @tenants.invite-by-login
  Scenario: An admin invites a person by GitHub login
    When an admin of a tenant invites a GitHub login
    Then the invite is bound to that account's GitHub id
    And the notice says which account was invited
    And that person can log in and sees the tenant

  @tenants.invite-says-what-it-opens
  Scenario: The invite form says what an invited person can do
    When an admin opens Members
    Then the form says that a member reads the tenant's repositories as Rubato's GitHub App, private ones included

  @tenants.only-admins-change-members
  Scenario: A member who is not an admin cannot invite, change roles or remove
    When a member who is not an admin opens Members
    Then they see no invite form and no remove button
    And the server refuses those requests from them

  @tenants.last-admin-stays
  Scenario: The last admin cannot be removed or made a member
    Given a tenant with one admin
    When that admin is removed or made a member
    Then it is refused, and the page says a tenant keeps at least one admin

  @tenants.remove-member
  Scenario: Removing a member takes a second press and ends what they had in the tenant
    When an admin presses Remove for a member and then confirms
    Then the member is gone from the list
    And their page for that tenant stops receiving its changes at once
    And their agents leave the tenant's rooms and what they queued for it is dropped

  @tenants.last-membership-ends-everything
  Scenario: A person removed from their last tenant is logged out and their companions are unpaired
    Given a person who is a member of one tenant
    When they are removed from it
    Then their sessions end and their companions are unpaired
    And they cannot log in again until someone invites them

  @tenants.refusal-in-words
  Scenario: A refused invite says why and keeps what was typed
    When an admin invites a login GitHub does not know
    Then the page says GitHub has no account with that login
    And the login is still in the box

  @tenants.live-only-own
  Scenario: A page hears only its own tenant's changes
    Given two people looking at different tenants
    When something changes in one tenant
    Then only the page showing that tenant is told

  @tenants.companion-serves-all-own
  Scenario: A companion gets the context graph of its owner's tenants and no other
    Given a person in the tenants "acme" and "bcorp" with a paired companion
    Then the companion is sent the context graph sources of acme and bcorp
    And a source of a tenant its owner is not in is not found
