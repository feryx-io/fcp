from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="TenantOffboarding")



@_attrs_define
class TenantOffboarding:
    """ What happens to a tenant's deleted user (`docs/USERS.md` §4): they are
    suspended at once and purged `grace_days` later.

        Attributes:
            grace_days (int | Unset): Days between a delete and the purge, so a mistake can be undone and
                a mailbox collected. 0 purges at once.
            meetings_owner (None | str | Unset): The username a purged user's meetings move to; without one they
                are left without an owner.
     """

    grace_days: int | Unset = UNSET
    meetings_owner: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        grace_days = self.grace_days

        meetings_owner: None | str | Unset
        if isinstance(self.meetings_owner, Unset):
            meetings_owner = UNSET
        else:
            meetings_owner = self.meetings_owner


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if grace_days is not UNSET:
            field_dict["grace_days"] = grace_days
        if meetings_owner is not UNSET:
            field_dict["meetings_owner"] = meetings_owner

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        grace_days = d.pop("grace_days", UNSET)

        def _parse_meetings_owner(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        meetings_owner = _parse_meetings_owner(d.pop("meetings_owner", UNSET))


        tenant_offboarding = cls(
            grace_days=grace_days,
            meetings_owner=meetings_owner,
        )


        tenant_offboarding.additional_properties = d
        return tenant_offboarding

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
