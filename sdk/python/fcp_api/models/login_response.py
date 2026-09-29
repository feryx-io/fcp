from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.auth_user_response import AuthUserResponse





T = TypeVar("T", bound="LoginResponse")



@_attrs_define
class LoginResponse:
    """ 
        Attributes:
            token (str):
            user (AuthUserResponse):
            password_expired (bool | None | Unset):
     """

    token: str
    user: AuthUserResponse
    password_expired: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        token = self.token

        user = self.user.to_dict()

        password_expired: bool | None | Unset
        if isinstance(self.password_expired, Unset):
            password_expired = UNSET
        else:
            password_expired = self.password_expired


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "token": token,
            "user": user,
        })
        if password_expired is not UNSET:
            field_dict["password_expired"] = password_expired

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.auth_user_response import AuthUserResponse # noqa: PLC0415
        d = dict(src_dict)
        token = d.pop("token")

        user = AuthUserResponse.from_dict(d.pop("user"))




        def _parse_password_expired(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        password_expired = _parse_password_expired(d.pop("password_expired", UNSET))


        login_response = cls(
            token=token,
            user=user,
            password_expired=password_expired,
        )


        login_response.additional_properties = d
        return login_response

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
