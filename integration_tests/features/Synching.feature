Feature: Synching

  Scenario: (B04) A node should sync in archival node
    Given I have a seed node MainSeed
    #When I mine 10 blocks on MainSeed
    Given I have a base node SyncNode connected to MainSeed
    Then node SyncNode is at height 10

    Scenario: (B05) A node should sync in pruned mode

Scenario: (B08) Download seeds from DNS

  Scenario: (B30) Switch from archival to pruned mode


