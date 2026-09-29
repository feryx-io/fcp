from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.join_role import JoinRole
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="MintJoinTokenRequest")



@_attrs_define
class MintJoinTokenRequest:
    """ `POST …/join-tokens` body.

        Attributes:
            display_name (None | str | Unset): The name the participant joins under (the meeting's name plus
                "guest"/"host" when absent).
            role (JoinRole | Unset): The role a token admits its holder in.
            ttl_secs (int | None | Unset): Lifetime in seconds; 15 minutes by default, one day at most.
     """

    display_name: None | str | Unset = UNSET
    role: JoinRole | Unset = UNSET
    ttl_secs: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name

        role: str | Unset = UNSET
        if not isinstance(self.role, Unset):
            role = self.role.value


        ttl_secs: int | None | Unset
        if isinstance(self.ttl_secs, Unset):
            ttl_secs = UNSET
        else:
            ttl_secs = self.ttl_secs


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if role is not UNSET:
            field_dict["role"] = role
        if ttl_secs is not UNSET:
            field_dict["ttl_secs"] = ttl_secs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        _role = d.pop("role", UNSET)
        role: JoinRole | Unset
        if isinstance(_role,  Unset):
            role = UNSET
        else:
            role = JoinRole(_role)




        def _parse_ttl_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        ttl_secs = _parse_ttl_secs(d.pop("ttl_secs", UNSET))


        mint_join_token_request = cls(
            display_name=display_name,
            role=role,
            ttl_secs=ttl_secs,
        )


        mint_join_token_request.additional_properties = d
        return mint_join_token_request

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
