from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.join_role import JoinRole
from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="JoinTokenResponse")



@_attrs_define
class JoinTokenResponse:
    """ A minted token.

        Attributes:
            token (str):
            room_number (str):
            display_name (str):
            role (JoinRole): The role a token admits its holder in.
            expires_at (datetime.datetime): Seconds since the Unix epoch.
            join_url (None | str | Unset): The gateway's join page with the token filled in, when
                `[conference] gateway_url` is configured.
     """

    token: str
    room_number: str
    display_name: str
    role: JoinRole
    expires_at: datetime.datetime
    join_url: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        token = self.token

        room_number = self.room_number

        display_name = self.display_name

        role = self.role.value

        expires_at = self.expires_at.isoformat()

        join_url: None | str | Unset
        if isinstance(self.join_url, Unset):
            join_url = UNSET
        else:
            join_url = self.join_url


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "token": token,
            "room_number": room_number,
            "display_name": display_name,
            "role": role,
            "expires_at": expires_at,
        })
        if join_url is not UNSET:
            field_dict["join_url"] = join_url

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        token = d.pop("token")

        room_number = d.pop("room_number")

        display_name = d.pop("display_name")

        role = JoinRole(d.pop("role"))




        expires_at = datetime.datetime.fromisoformat(d.pop("expires_at"))




        def _parse_join_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        join_url = _parse_join_url(d.pop("join_url", UNSET))


        join_token_response = cls(
            token=token,
            room_number=room_number,
            display_name=display_name,
            role=role,
            expires_at=expires_at,
            join_url=join_url,
        )


        join_token_response.additional_properties = d
        return join_token_response

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
