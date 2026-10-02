from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.password_login import PasswordLogin
from ..types import UNSET, Unset






T = TypeVar("T", bound="TenantSignIn")



@_attrs_define
class TenantSignIn:
    """ How a tenant's people may sign in (`docs/IDENTITY_AND_DEVICES.md` §2).

        Attributes:
            password_login (PasswordLogin | Unset): Who may sign in with a password.
     """

    password_login: PasswordLogin | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        password_login: str | Unset = UNSET
        if not isinstance(self.password_login, Unset):
            password_login = self.password_login.value



        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if password_login is not UNSET:
            field_dict["password_login"] = password_login

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        _password_login = d.pop("password_login", UNSET)
        password_login: PasswordLogin | Unset
        if isinstance(_password_login,  Unset):
            password_login = UNSET
        else:
            password_login = PasswordLogin(_password_login)




        tenant_sign_in = cls(
            password_login=password_login,
        )


        tenant_sign_in.additional_properties = d
        return tenant_sign_in

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
