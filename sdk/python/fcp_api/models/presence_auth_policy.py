from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.presence_auth_policy_event_allow import PresenceAuthPolicyEventAllow
  from ..models.presence_auth_policy_event_block import PresenceAuthPolicyEventBlock
  from ..models.time_window import TimeWindow





T = TypeVar("T", bound="PresenceAuthPolicy")



@_attrs_define
class PresenceAuthPolicy:
    """ Authorization policy for presence subscriptions.

    Determines which watchers are allowed to subscribe to a presentity's state.
    If no policy is set for a presentity, all watchers are allowed (default open).

    Evaluation order (first match wins):
    1. Block list (AOR exact match) → deny
    2. Per-event block list → deny
    3. Block patterns (domain wildcard) → deny
    4. Allow list (AOR exact match) → allow
    5. Per-event allow list → allow (if non-empty, restricts to listed AORs)
    6. Allow patterns (domain wildcard) → allow
    7. Time window check → deny if outside windows
    8. Default

        Attributes:
            default_allow (bool): If `true`, watchers not in either list are allowed. If `false`, only
                watchers on the allow_list are permitted.
            allow_list (list[str] | Unset): Watcher AORs explicitly allowed to subscribe.
            block_list (list[str] | Unset): Watcher AORs explicitly blocked from subscribing.
            event_allow (PresenceAuthPolicyEventAllow | Unset): Event-package-specific allow lists. Key: event package name.
                If a watcher's event package has an entry here, only listed AORs
                may subscribe for that specific package.
            event_block (PresenceAuthPolicyEventBlock | Unset): Event-package-specific block lists.
            time_windows (list[TimeWindow] | Unset): Optional time windows during which subscriptions are allowed.
                Empty = always allowed.
            allow_patterns (list[str] | Unset): Group-based allow patterns. Supports domain wildcards like `*@domain`.
            block_patterns (list[str] | Unset): Group-based block patterns.
     """

    default_allow: bool
    allow_list: list[str] | Unset = UNSET
    block_list: list[str] | Unset = UNSET
    event_allow: PresenceAuthPolicyEventAllow | Unset = UNSET
    event_block: PresenceAuthPolicyEventBlock | Unset = UNSET
    time_windows: list[TimeWindow] | Unset = UNSET
    allow_patterns: list[str] | Unset = UNSET
    block_patterns: list[str] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.presence_auth_policy_event_allow import PresenceAuthPolicyEventAllow # noqa: PLC0415
        from ..models.presence_auth_policy_event_block import PresenceAuthPolicyEventBlock # noqa: PLC0415
        from ..models.time_window import TimeWindow # noqa: PLC0415
        default_allow = self.default_allow

        allow_list: list[str] | Unset = UNSET
        if not isinstance(self.allow_list, Unset):
            allow_list = self.allow_list



        block_list: list[str] | Unset = UNSET
        if not isinstance(self.block_list, Unset):
            block_list = self.block_list



        event_allow: dict[str, Any] | Unset = UNSET
        if not isinstance(self.event_allow, Unset):
            event_allow = self.event_allow.to_dict()

        event_block: dict[str, Any] | Unset = UNSET
        if not isinstance(self.event_block, Unset):
            event_block = self.event_block.to_dict()

        time_windows: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.time_windows, Unset):
            time_windows = []
            for time_windows_item_data in self.time_windows:
                time_windows_item = time_windows_item_data.to_dict()
                time_windows.append(time_windows_item)



        allow_patterns: list[str] | Unset = UNSET
        if not isinstance(self.allow_patterns, Unset):
            allow_patterns = self.allow_patterns



        block_patterns: list[str] | Unset = UNSET
        if not isinstance(self.block_patterns, Unset):
            block_patterns = self.block_patterns




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "default_allow": default_allow,
        })
        if allow_list is not UNSET:
            field_dict["allow_list"] = allow_list
        if block_list is not UNSET:
            field_dict["block_list"] = block_list
        if event_allow is not UNSET:
            field_dict["event_allow"] = event_allow
        if event_block is not UNSET:
            field_dict["event_block"] = event_block
        if time_windows is not UNSET:
            field_dict["time_windows"] = time_windows
        if allow_patterns is not UNSET:
            field_dict["allow_patterns"] = allow_patterns
        if block_patterns is not UNSET:
            field_dict["block_patterns"] = block_patterns

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.presence_auth_policy_event_allow import PresenceAuthPolicyEventAllow # noqa: PLC0415
        from ..models.presence_auth_policy_event_block import PresenceAuthPolicyEventBlock # noqa: PLC0415
        from ..models.time_window import TimeWindow # noqa: PLC0415
        d = dict(src_dict)
        default_allow = d.pop("default_allow")

        allow_list = cast(list[str], d.pop("allow_list", UNSET))


        block_list = cast(list[str], d.pop("block_list", UNSET))


        _event_allow = d.pop("event_allow", UNSET)
        event_allow: PresenceAuthPolicyEventAllow | Unset
        if isinstance(_event_allow,  Unset):
            event_allow = UNSET
        else:
            event_allow = PresenceAuthPolicyEventAllow.from_dict(_event_allow)




        _event_block = d.pop("event_block", UNSET)
        event_block: PresenceAuthPolicyEventBlock | Unset
        if isinstance(_event_block,  Unset):
            event_block = UNSET
        else:
            event_block = PresenceAuthPolicyEventBlock.from_dict(_event_block)




        _time_windows = d.pop("time_windows", UNSET)
        time_windows: list[TimeWindow] | Unset = UNSET
        if _time_windows is not UNSET:
            time_windows = []
            for time_windows_item_data in _time_windows:
                time_windows_item = TimeWindow.from_dict(time_windows_item_data)



                time_windows.append(time_windows_item)


        allow_patterns = cast(list[str], d.pop("allow_patterns", UNSET))


        block_patterns = cast(list[str], d.pop("block_patterns", UNSET))


        presence_auth_policy = cls(
            default_allow=default_allow,
            allow_list=allow_list,
            block_list=block_list,
            event_allow=event_allow,
            event_block=event_block,
            time_windows=time_windows,
            allow_patterns=allow_patterns,
            block_patterns=block_patterns,
        )


        presence_auth_policy.additional_properties = d
        return presence_auth_policy

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
